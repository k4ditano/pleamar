//! An owned Windows media session for controls/capability/Unicode validation.
//! It publishes a test timeline and receives real SMTC commands; no audio plays.
#[cfg(not(target_os = "windows"))]
fn main() { eprintln!("media-fixture requires Windows"); }
#[cfg(target_os = "windows")]
fn main() { if let Err(error) = fixture::run() { eprintln!("{error}"); std::process::exit(1); } }

#[cfg(target_os = "windows")]
mod fixture {
    use std::sync::{Mutex, OnceLock};
    use windows::{core::{w, HSTRING}, Foundation::TypedEventHandler,
        Media::{SystemMediaTransportControls as Controls, SystemMediaTransportControlsButton as Button,
            SystemMediaTransportControlsButtonPressedEventArgs as Pressed, MediaPlaybackStatus, MediaPlaybackType},
        Win32::{Foundation::*, Graphics::Gdi::*, System::{LibraryLoader::GetModuleHandleW,
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED, ISystemMediaTransportControlsInterop}},
            UI::{Shell::SetCurrentProcessExplicitAppUserModelID, WindowsAndMessaging::*}}};
    const BUTTON: u32 = WM_APP + 1;
    struct State { controls: Controls, track: usize, playing: bool, events: u32 }
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    fn publish(hwnd: HWND, state: &State) -> windows::core::Result<()> {
        let title = if state.track == 0 { "Pleamar media validation — Español" } else { "Pleamar media validation — 日本語" };
        state.controls.SetPlaybackStatus(if state.playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused })?;
        state.controls.SetIsPlayEnabled(true)?;
        state.controls.SetIsPauseEnabled(true)?;
        state.controls.SetIsPreviousEnabled(state.track > 0)?;
        state.controls.SetIsNextEnabled(state.track == 0)?;
        let display = state.controls.DisplayUpdater()?;
        display.SetType(MediaPlaybackType::Music)?;
        let music = display.MusicProperties()?;
        music.SetTitle(&HSTRING::from(title))?;
        music.SetArtist(&HSTRING::from("Owned native fixture — no audio"))?;
        music.SetAlbumTitle(&HSTRING::from("Windows validation"))?;
        display.Update()?;
        let status = if state.playing { "Playing" } else { "Paused" };
        unsafe {
            SetWindowTextW(hwnd, &HSTRING::from(format!("Pleamar media fixture — {status} — track {} — {} events", state.track + 1, state.events)))?;
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
        println!("fixture: state track={} playing={} previous={} next={} events={}", state.track + 1, state.playing, state.track > 0, state.track == 0, state.events);
        Ok(())
    }
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { unsafe {
        match message {
            BUTTON => {
                if let Some(state) = STATE.get() {
                    let mut state = state.lock().unwrap();
                    let button = Button(wp.0 as i32);
                    match button {
                        Button::Play => state.playing = true,
                        Button::Pause => state.playing = false,
                        Button::Next if state.track == 0 => state.track = 1,
                        Button::Previous if state.track > 0 => state.track = 0,
                        _ => { println!("fixture: unexpected button {}", button.0); return LRESULT(0); }
                    }
                    state.events += 1;
                    println!("fixture: received button={}", button.0);
                    if let Err(error) = publish(hwnd, &state) { eprintln!("fixture: publish failed: {error}"); }
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut area = RECT { left: 20, top: 20, right: 530, bottom: 150 };
                let mut text: Vec<u16> = "Native SMTC test session. No audio is played.\nUse Marea to pause/play, move to track 2 and return.\nPrevious is disabled on track 1; Next on track 2.\nClose this window to remove the session.".encode_utf16().collect();
                DrawTextW(dc, &mut text, &mut area, DT_LEFT | DT_WORDBREAK);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_TIMER | WM_CLOSE => { let _ = DestroyWindow(hwnd); LRESULT(0) }
            WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    } }
    struct SilentAudio(windows::Win32::Media::Audio::IAudioClient);
    impl Drop for SilentAudio { fn drop(&mut self) { unsafe { let _ = self.0.Stop(); } } }
    unsafe fn silent_audio() -> windows::core::Result<SilentAudio> { unsafe {
        use windows::Win32::{Media::Audio::*, System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL}};
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = e.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let initialized = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 1_000_000, 0, format, None);
        CoTaskMemFree(Some(format as _));
        initialized?;
        let render: IAudioRenderClient = client.GetService()?;
        let size = client.GetBufferSize()?;
        render.GetBuffer(size)?;
        render.ReleaseBuffer(size, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        client.Start()?;
        println!("fixture: silent audio session ready");
        Ok(SilentAudio(client))
    } }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> { unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
        struct Apartment;
        impl Drop for Apartment { fn drop(&mut self) { unsafe { RoUninitialize(); } } }
        let _apartment = Apartment;
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).is_some_and(|arg| arg == "--read") {
            // Independent WinRT readback; the runtime under test is queried
            // separately through Luau and its native service implementation.
            let manager = windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
            let session = manager.GetCurrentSession()?;
            let media = session.TryGetMediaPropertiesAsync()?.join()?;
            let playback = session.GetPlaybackInfo()?;
            let controls = playback.Controls()?;
            println!("{}", serde_json::json!({"player":session.SourceAppUserModelId()?.to_string(),
                "title":media.Title()?.to_string(), "artist":media.Artist()?.to_string(),
                "status":playback.PlaybackStatus()?.0, "play":controls.IsPlayEnabled()?,
                "pause":controls.IsPauseEnabled()?, "toggle":controls.IsPlayPauseToggleEnabled()?,
                "next":controls.IsNextEnabled()?, "previous":controls.IsPreviousEnabled()?}));
            return Ok(());
        }
        let app_id = format!("org.pleamar.validation.media.{}", std::process::id());
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(&app_id))?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("PleamarMediaFixture");
        let wc = WNDCLASSW { hInstance: instance.into(), lpszClassName: class, lpfnWndProc: Some(procedure),
            hCursor: LoadCursorW(None, IDC_ARROW)?, hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as _), ..Default::default() };
        if RegisterClassW(&wc) == 0 { return Err(windows::core::Error::from_thread().into()); }
        let hwnd = CreateWindowExW(WS_EX_APPWINDOW, class, w!("Pleamar media fixture"), WS_OVERLAPPEDWINDOW,
            40, 300, 580, 220, None, None, Some(instance.into()), None)?;
        // Explicit window identity also exercises custom classic application IDs.
        {
            use windows::Win32::{UI::Shell::PropertiesSystem::{SHGetPropertyStoreForWindow, IPropertyStore}, System::Com::StructuredStorage::PROPVARIANT};
            let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
            let value = PROPVARIANT::from(app_id.as_str());
            let key = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };
            store.SetValue(&key, &value)?;
            store.Commit()?;
        }
        let _audio = if args.iter().any(|arg| arg == "--audio") { Some(silent_audio()?) } else { None };
        let interop: ISystemMediaTransportControlsInterop = windows::core::factory::<Controls, _>()?;
        let controls: Controls = interop.GetForWindow(hwnd)?;
        controls.SetIsEnabled(true)?;
        let handle = hwnd.0 as isize;
        let token = controls.ButtonPressed(&TypedEventHandler::<Controls, Pressed>::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                PostMessageW(Some(HWND(handle as _)), BUTTON, WPARAM(args.Button()?.0 as usize), LPARAM(0))?;
            }
            Ok(())
        }))?;
        STATE.set(Mutex::new(State { controls: controls.clone(), track: 0, playing: true, events: 0 })).map_err(|_| "fixture already initialized")?;
        publish(hwnd, &STATE.get().unwrap().lock().unwrap())?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), 1, 15 * 60 * 1000, None);
        println!("fixture: ready player={app_id}");
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() { let _ = TranslateMessage(&message); DispatchMessageW(&message); }
        controls.RemoveButtonPressed(token)?;
        controls.SetIsEnabled(false)?;
        println!("fixture: closed");
        Ok(())
    } }
}
