//! Sound a scene makes itself: a jump, a coin, a tune under a game.
//!
//! One mixer for the whole program, on its own thread. It opens one stream to
//! the sound server —PulseAudio's protocol, which PipeWire speaks too— the
//! first time something is asked to sound, mixes there whatever is playing
//! each time the server asks for more, and lets the stream go after a few
//! seconds of silence: a scene that makes no sound holds nothing open.
//!
//! What it plays: a WAV or an Ogg Vorbis file, read once and kept, and tones
//! made here —a wave, from one pitch to another, with its rise and its fall—,
//! which is most of what a small game needs and wants no files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What everything is mixed at, and sent as: two channels of 16 bits.
const RATE: u32 = 48_000;
/// How much sound waits in the server's buffer: what is heard comes that
/// long after it is asked for, at most.
const LATENCY: Duration = Duration::from_millis(40);
/// With nothing sounding for this long, the stream is closed.
const IDLE: Duration = Duration::from_secs(4);
/// The longest tone, and how many things sound at once at most: one more
/// takes the place of the one that started first.
const LONGEST_TONE: f32 = 10.0;
const VOICES: usize = 48;

/// A sound, read: two channels interleaved, at its own rate.
pub struct Clip {
    rate: u32,
    samples: Vec<i16>,
}

/// How a sound is played.
#[derive(Clone, Copy)]
pub struct How {
    /// 1 is as it is.
    pub volume: f32,
    /// 1 is as it is; 2, an octave up and half as long.
    pub pitch: f32,
    /// −1 all to the left, 1 all to the right.
    pub pan: f32,
    pub repeats: bool,
}

impl Default for How {
    fn default() -> How {
        How { volume: 1.0, pitch: 1.0, pan: 0.0, repeats: false }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Wave {
    Sine,
    Square,
    Saw,
    Triangle,
    Noise,
}

/// A tone made here: `wave`, from `from` Hz to `to` Hz over `seconds`, rising
/// in `attack` and falling in `release`.
pub struct Tone {
    pub wave: Wave,
    pub from: f32,
    pub to: f32,
    pub seconds: f32,
    pub attack: f32,
    pub release: f32,
    /// Of a square wave: how much of each turn it is up (0.5, an even one).
    pub duty: f32,
}

enum Order {
    Play(u32, Arc<Clip>, How),
    /// `None`: everything.
    Stop(Option<u32>),
    Volume(f32),
}

struct Voice {
    id: u32,
    clip: Arc<Clip>,
    /// Where it is in the clip, in its frames, and how many it moves on for each one sent.
    at: f64,
    step: f64,
    left: f32,
    right: f32,
    repeats: bool,
}

struct Mixer {
    orders: Sender<Order>,
    next: u32,
    /// What has been read from disk, by its path and when it was written: a
    /// file saved again is read again.
    clips: HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<Clip>)>,
}

static MIXER: Mutex<Option<Mixer>> = Mutex::new(None);

fn with_mixer<T>(f: impl FnOnce(&mut Mixer) -> T) -> T {
    let mut m = MIXER.lock().unwrap();
    let mixer = m.get_or_insert_with(|| {
        let (orders, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("sound".into()).spawn(move || run(rx));
        Mixer { orders, next: 0, clips: HashMap::new() }
    });
    f(mixer)
}

/// Plays a file. Answers with its number, to stop it by.
pub fn play(path: &Path, how: How) -> Result<u32, String> {
    let written = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let known = with_mixer(|m| m.clips.get(path).filter(|(w, _)| *w == written).map(|(_, c)| c.clone()));
    let clip = match known {
        Some(c) => c,
        None => {
            let c = Arc::new(read(path)?);
            with_mixer(|m| m.clips.insert(path.to_owned(), (written, c.clone())));
            c
        }
    };
    Ok(start(clip, how))
}

/// Reads a file ahead of time, so the first time it sounds it does not wait for the disk.
pub fn load(path: &Path) -> Result<(), String> {
    let written = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if with_mixer(|m| m.clips.get(path).is_some_and(|(w, _)| *w == written)) {
        return Ok(());
    }
    let c = Arc::new(read(path)?);
    with_mixer(|m| m.clips.insert(path.to_owned(), (written, c)));
    Ok(())
}

/// Plays a tone made here.
pub fn tone(t: &Tone, how: How) -> u32 {
    start(Arc::new(synth(t)), how)
}

pub fn stop(id: Option<u32>) {
    if let Some(m) = MIXER.lock().unwrap().as_ref() {
        let _ = m.orders.send(Order::Stop(id));
    }
}

/// How loud everything this program plays is: 1 as it is.
pub fn volume(v: f32) {
    with_mixer(|m| m.orders.send(Order::Volume(v.clamp(0.0, 4.0))).is_ok());
}

fn start(clip: Arc<Clip>, how: How) -> u32 {
    with_mixer(|m| {
        m.next += 1;
        let _ = m.orders.send(Order::Play(m.next, clip, how));
        m.next
    })
}

// ── reading ───────────────────────────────────────────────────────

fn read(path: &Path) -> Result<Clip, String> {
    let said = |e: String| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| said(e.to_string()))?;
    if bytes.starts_with(b"RIFF") {
        wav(&bytes).map_err(|e| said(e.to_owned()))
    } else if bytes.starts_with(b"OggS") {
        vorbis(bytes).map_err(said)
    } else {
        Err(said("neither a WAV nor an Ogg Vorbis file, which are the two it reads".into()))
    }
}

/// A WAV: whole numbers of 8, 16, 24 or 32 bits, or 32 bits with a point.
fn wav(b: &[u8]) -> Result<Clip, &'static str> {
    let u16_at = |k: usize| b.get(k..k + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |k: usize| b.get(k..k + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    if b.get(8..12) != Some(b"WAVE") {
        return Err("it says RIFF but it is not a WAV");
    }
    let (mut format, mut data) = (None, None);
    let mut k = 12;
    while let (Some(name), Some(len)) = (b.get(k..k + 4), u32_at(k + 4)) {
        let body = k + 8;
        let end = (body + len as usize).min(b.len());
        match name {
            b"fmt " => format = Some(body),
            b"data" => data = Some(&b[body..end]),
            _ => {}
        }
        k = body + len as usize + (len as usize & 1);
    }
    let (Some(f), Some(data)) = (format, data) else { return Err("a WAV with no format or no sound in it") };
    let (Some(mut kind), Some(channels), Some(rate), Some(bits)) = (u16_at(f), u16_at(f + 2), u32_at(f + 4), u16_at(f + 14)) else {
        return Err("a WAV cut short");
    };
    // "Extensible": the real kind is further in.
    if kind == 0xfffe {
        kind = u16_at(f + 24).unwrap_or(0);
    }
    let (channels, bytes) = (channels as usize, bits as usize / 8);
    if channels == 0 || rate == 0 || !matches!((kind, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
        return Err("a WAV of a kind it does not read: only plain PCM, of 8, 16, 24 or 32 bits");
    }
    let one = |s: &[u8]| -> i16 {
        match (kind, bytes) {
            (1, 1) => (s[0] as i16 - 128) << 8,
            (1, 2) => i16::from_le_bytes([s[0], s[1]]),
            (1, 3) => i16::from_le_bytes([s[1], s[2]]),
            (1, _) => i16::from_le_bytes([s[2], s[3]]),
            _ => (f32::from_le_bytes([s[0], s[1], s[2], s[3]]).clamp(-1.0, 1.0) * 32767.0) as i16,
        }
    };
    let mut samples = Vec::with_capacity(data.len() / (bytes * channels) * 2);
    for frame in data.chunks_exact(bytes * channels) {
        let l = one(&frame[..bytes]);
        // One channel goes to both sides; of more than two, the first two.
        let r = if channels > 1 { one(&frame[bytes..bytes * 2]) } else { l };
        samples.extend([l, r]);
    }
    Ok(Clip { rate, samples })
}

fn vorbis(bytes: Vec<u8>) -> Result<Clip, String> {
    let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let (channels, rate) = (r.ident_hdr.audio_channels as usize, r.ident_hdr.audio_sample_rate);
    if channels == 0 || rate == 0 {
        return Err("an Ogg with no sound in it".into());
    }
    let mut samples = Vec::new();
    while let Some(packet) = r.read_dec_packet_itl().map_err(|e| e.to_string())? {
        for frame in packet.chunks_exact(channels) {
            samples.extend([frame[0], if channels > 1 { frame[1] } else { frame[0] }]);
        }
    }
    Ok(Clip { rate, samples })
}

/// A tone, worked out sample by sample.
fn synth(t: &Tone) -> Clip {
    let seconds = t.seconds.clamp(0.005, LONGEST_TONE);
    let n = (seconds * RATE as f32) as usize;
    let (attack, release) = (t.attack.clamp(0.0, seconds), t.release.clamp(0.0, seconds));
    let mut samples = Vec::with_capacity(n * 2);
    let mut phase = 0.0f32;
    // Noise: a new level at each turn of the pitch, so a low one rumbles and a high one hisses.
    let (mut seed, mut level, mut turn) = (0x2545f491u32, 0.0f32, 1.0f32);
    for k in 0..n {
        let at = k as f32 / RATE as f32;
        let along = k as f32 / n.max(1) as f32;
        // The pitch slides evenly as the ear hears it: by ratio, not by hertz.
        let hz = if t.from > 0.0 && t.to > 0.0 { t.from * (t.to / t.from).powf(along) } else { t.from.max(1.0) };
        phase = (phase + hz / RATE as f32).fract();
        let v = match t.wave {
            Wave::Sine => (phase * std::f32::consts::TAU).sin(),
            Wave::Square => if phase < t.duty.clamp(0.05, 0.95) { 0.7 } else { -0.7 },
            Wave::Saw => (phase * 2.0 - 1.0) * 0.8,
            Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Wave::Noise => {
                turn += hz * 8.0 / RATE as f32;
                if turn >= 1.0 {
                    turn -= turn.floor();
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    level = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
                }
                level
            }
        };
        let rise = if attack > 0.0 { (at / attack).min(1.0) } else { 1.0 };
        let fall = if release > 0.0 { ((seconds - at) / release).min(1.0) } else { 1.0 };
        let s = (v * rise * fall * 0.6 * 32767.0) as i16;
        samples.extend([s, s]);
    }
    Clip { rate: RATE, samples }
}

// ── mixing ────────────────────────────────────────────────────────

/// The mixer's thread: asleep until something is asked to sound, then a
/// stream to the server for as long as there is something to play.
fn run(orders: Receiver<Order>) {
    let mut voices: Vec<Voice> = Vec::new();
    let mut loud = 1.0f32;
    let mut failed = false;
    loop {
        // Nothing sounds: wait for an order, however long it takes.
        if voices.is_empty() {
            match orders.recv() {
                Ok(o) => take(&mut voices, &mut loud, o),
                Err(_) => return,
            }
            continue;
        }
        if let Err(e) = stream(&orders, &mut voices, &mut loud) {
            // Said once: a machine with no sound server asks for it again at every jump.
            if !std::mem::replace(&mut failed, true) {
                eprintln!("sound  · {e}: nothing will be heard");
            }
            voices.clear();
        }
    }
}

fn take(voices: &mut Vec<Voice>, loud: &mut f32, o: Order) {
    match o {
        Order::Play(id, clip, how) => {
            if clip.samples.len() < 2 {
                return;
            }
            if voices.len() >= VOICES {
                voices.remove(0);
            }
            // The same loudness whichever side it is on: what one loses the other gains.
            let angle = (how.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let v = how.volume.clamp(0.0, 4.0) * std::f32::consts::SQRT_2;
            voices.push(Voice { id, at: 0.0, step: clip.rate as f64 / RATE as f64 * how.pitch.clamp(0.05, 16.0) as f64, left: v * angle.cos(), right: v * angle.sin(), repeats: how.repeats, clip });
        }
        Order::Stop(Some(id)) => voices.retain(|v| v.id != id),
        Order::Stop(None) => voices.clear(),
        Order::Volume(v) => *loud = v,
    }
}

/// Fills `out` (bytes: left and right, 16 bits each) with what is sounding.
fn mix(voices: &mut Vec<Voice>, loud: f32, out: &mut [u8]) {
    for frame in out.chunks_exact_mut(4) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for v in voices.iter_mut() {
            let frames = v.clip.samples.len() / 2;
            if v.at >= frames as f64 {
                if !v.repeats {
                    continue;
                }
                v.at %= frames as f64;
            }
            // Between one sample and the next, in proportion.
            let k = v.at as usize;
            let next = if k + 1 < frames { k + 1 } else if v.repeats { 0 } else { k };
            let f = (v.at - k as f64) as f32;
            let s = &v.clip.samples;
            l += (s[k * 2] as f32 * (1.0 - f) + s[next * 2] as f32 * f) * v.left;
            r += (s[k * 2 + 1] as f32 * (1.0 - f) + s[next * 2 + 1] as f32 * f) * v.right;
            v.at += v.step;
        }
        // Too many at once does not crackle: past three quarters it is eased into the top.
        let ease = |x: f32| {
            let x = x * loud / 32767.0;
            let soft = if x.abs() <= 0.75 { x } else { x.signum() * (0.75 + 0.25 * ((x.abs() - 0.75) / 0.25).tanh()) };
            (soft * 32767.0) as i16
        };
        frame[..2].copy_from_slice(&ease(l).to_le_bytes());
        frame[2..].copy_from_slice(&ease(r).to_le_bytes());
    }
    voices.retain(|v| v.repeats || v.at < (v.clip.samples.len() / 2) as f64);
}

/// One stream to the server, fed each time it asks, until there has been
/// nothing to play for a while.
#[cfg(target_os = "linux")]
fn stream(orders: &Receiver<Order>, voices: &mut Vec<Voice>, loud: &mut f32) -> Result<(), String> {
    use pulseaudio::protocol::{self, Command};
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;
    let said = |e: protocol::ProtocolError| e.to_string();
    let path = pulseaudio::socket_path_from_env().ok_or("no sound server")?;
    let mut sock = BufReader::new(UnixStream::connect(path).map_err(|e| e.to_string())?);
    let cookie = pulseaudio::cookie_path_from_env().and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
    let auth = protocol::AuthParams { version: protocol::MAX_VERSION, supports_shm: false, supports_memfd: false, cookie };
    protocol::write_command_message(sock.get_mut(), 0, &Command::Auth(auth), protocol::MAX_VERSION).map_err(said)?;
    let (_, reply) = protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION).map_err(said)?;
    let version = protocol::MAX_VERSION.min(reply.version);
    let mut props = protocol::Props::new();
    props.set(protocol::Prop::ApplicationName, c"pleamar".to_owned());
    protocol::write_command_message(sock.get_mut(), 1, &Command::SetClientName(props), version).map_err(said)?;
    protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version).map_err(said)?;

    let wanted = (RATE as f32 * 4.0 * LATENCY.as_secs_f32()) as u32 & !3;
    let mut props = protocol::Props::new();
    props.set(protocol::Prop::MediaName, c"scene".to_owned());
    let params = protocol::PlaybackStreamParams {
        sample_spec: protocol::SampleSpec { format: protocol::SampleFormat::S16Le, channels: 2, sample_rate: RATE },
        channel_map: protocol::ChannelMap::stereo(),
        cvolume: Some(protocol::ChannelVolume::norm(2)),
        sink_name: Some(protocol::DEFAULT_SINK.to_owned()),
        buffer_attr: protocol::stream::BufferAttr { target_length: wanted, pre_buffering: 0, minimum_request_length: wanted / 4, ..Default::default() },
        flags: protocol::stream::StreamFlags { adjust_latency: true, ..Default::default() },
        props,
        ..Default::default()
    };
    protocol::write_command_message(sock.get_mut(), 2, &Command::CreatePlaybackStream(params), version).map_err(said)?;
    let (_, info) = protocol::read_reply_message::<protocol::CreatePlaybackStreamReply>(&mut sock, version).map_err(said)?;

    let mut buffer: Vec<u8> = Vec::new();
    let mut quiet_since: Option<Instant> = None;
    let mut send = |sock: &mut BufReader<UnixStream>, voices: &mut Vec<Voice>, loud: &mut f32, length: usize| -> Result<bool, String> {
        loop {
            match orders.try_recv() {
                Ok(o) => take(voices, loud, o),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(false),
            }
        }
        // Silence is sent too —the server would otherwise count it as a
        // stall—, but not for ever.
        match (voices.is_empty(), quiet_since) {
            (false, _) => quiet_since = None,
            (true, None) => quiet_since = Some(Instant::now()),
            (true, Some(t)) if t.elapsed() > IDLE => return Ok(false),
            _ => {}
        }
        buffer.clear();
        buffer.resize(length & !3, 0);
        mix(voices, *loud, &mut buffer);
        protocol::write_memblock(sock.get_mut(), info.channel, &buffer, 0).map_err(said)?;
        Ok(true)
    };
    if !send(&mut sock, voices, loud, info.requested_bytes as usize)? {
        return Ok(());
    }
    loop {
        let (_, message) = protocol::read_command_message(&mut sock, version).map_err(said)?;
        match message {
            Command::Request(r) if r.channel == info.channel => {
                if !send(&mut sock, voices, loud, r.length as usize)? {
                    return Ok(());
                }
            }
            Command::Error(e) => return Err(format!("the sound server said {e:?}")),
            _ => {}
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn stream(_: &Receiver<Order>, _: &mut Vec<Voice>, _: &mut f32) -> Result<(), String> {
    let _ = (LATENCY, IDLE, mix as fn(&mut Vec<Voice>, f32, &mut [u8]), TryRecvError::Empty);
    Err("playing sound is not written for this system yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_is_read_whatever_its_bits() {
        let mut b = Vec::new();
        b.extend(b"RIFF\0\0\0\0WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(22050u32.to_le_bytes());
        b.extend(22050u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(8u16.to_le_bytes());
        b.extend(b"data");
        b.extend(3u32.to_le_bytes());
        b.extend([128u8, 255, 0]);
        let c = wav(&b).unwrap();
        assert_eq!(c.rate, 22050);
        // One channel, to both sides.
        assert_eq!(c.samples, vec![0, 0, 127 << 8, 127 << 8, -128 << 8, -128 << 8]);
        assert!(wav(b"RIFF\0\0\0\0AVI ").is_err());
    }

    #[test]
    fn an_ogg_is_read_if_this_machine_has_one() {
        // The desktop's own sounds, where there are any: without them, nothing to check.
        let path = Path::new("/usr/share/sounds/freedesktop/stereo/bell.oga");
        if path.is_file() {
            let c = read(path).unwrap();
            assert!(c.rate >= 8000 && c.samples.len() > 2000 && c.samples.len() % 2 == 0);
            assert!(c.samples.iter().any(|s| s.abs() > 500));
        }
        assert!(read(Path::new("/nowhere/at-all.ogg")).is_err());
    }

    #[test]
    fn a_tone_lasts_what_it_says_and_ends_in_silence() {
        let c = synth(&Tone { wave: Wave::Square, from: 440.0, to: 880.0, seconds: 0.1, attack: 0.005, release: 0.03, duty: 0.5 });
        assert_eq!(c.samples.len(), 4800 * 2);
        assert_eq!(c.samples[0], 0);
        assert!(c.samples[c.samples.len() - 2].abs() < 200);
        assert!(c.samples.iter().any(|s| s.abs() > 8000));
    }

    #[test]
    fn what_is_mixed_ends_and_what_repeats_goes_on() {
        let clip = Arc::new(Clip { rate: RATE, samples: vec![1000; 8] });
        let mut voices = Vec::new();
        let mut loud = 1.0;
        take(&mut voices, &mut loud, Order::Play(1, clip.clone(), How::default()));
        take(&mut voices, &mut loud, Order::Play(2, clip, How { repeats: true, ..Default::default() }));
        let mut out = vec![0u8; 4 * 6];
        mix(&mut voices, loud, &mut out);
        // The first four frames, both; the last two, only the one that repeats.
        let first = i16::from_le_bytes([out[0], out[1]]);
        let last = i16::from_le_bytes([out[20], out[21]]);
        assert!(first > last && last > 0);
        assert_eq!(voices.len(), 1);
        take(&mut voices, &mut loud, Order::Stop(None));
        assert!(voices.is_empty());
    }
}
