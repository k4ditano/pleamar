//! Popup anchors stay in client coordinates while native placement follows the owner.
use super::*;
use windows::Win32::Graphics::Gdi::MonitorFromPoint;

fn slide(at: POINT, size: (i32, i32), scale: f32, work: RECT) -> Result<RECT, String> {
    if !scale.is_finite() || scale <= 0.0 || work.right <= work.left || work.bottom <= work.top {
        return Err("invalid popup output geometry".into());
    }
    let extent = |n: i32| -> Result<i64, String> {
        let value = (n.max(1) as f64 * scale as f64).round().max(1.0);
        if value > i32::MAX as f64 { return Err("popup extent exceeds native coordinates".into()); }
        Ok(value as i64)
    };
    let (width, height) = (extent(size.0)?, extent(size.1)?);
    // Like the Wayland SlideX/SlideY constraint, preserve the declared size.
    // An oversized menu starts at the leading edge; its scene must provide scrolling.
    let x = i64::from(at.x).clamp(i64::from(work.left), (i64::from(work.right) - width).max(i64::from(work.left)));
    let y = i64::from(at.y).clamp(i64::from(work.top), (i64::from(work.bottom) - height).max(i64::from(work.top)));
    let coordinate = |v| i32::try_from(v).map_err(|_| "popup position exceeds native coordinates".to_owned());
    Ok(RECT { left:coordinate(x)?, top:coordinate(y)?, right:coordinate(x+width)?, bottom:coordinate(y+height)? })
}

pub(super) fn placement(parent: &WindowState, bounds: [i32; 4]) -> Result<Monitor, String> {
    let [x,y,w,h] = bounds;
    let mut corner = POINT { x:px(x,parent.scale()), y:px(y,parent.scale()) };
    if !unsafe { ClientToScreen(parent.hwnd(), &mut corner) }.as_bool() {
        return Err("popup owner has no client origin".into());
    }
    let handle = unsafe { MonitorFromPoint(corner, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return Err("popup output is no longer available".into());
    }
    let end = info.szDevice.iter().position(|c| *c==0).unwrap_or(info.szDevice.len());
    let scale = monitor_scale(handle);
    Ok(Monitor { rect:slide(corner,(w,h),scale,info.monitorInfo.rcWork)?,
        name:String::from_utf16_lossy(&info.szDevice[..end]), scale, mhz:0 })
}

pub(super) struct Tracked {
    parent: Weak<WindowState>,
    popup: Weak<WindowState>,
    bounds: [i32; 4],
}

impl Tracked {
    pub fn new(parent: &Arc<WindowState>, popup: &Arc<WindowState>, bounds: [i32;4]) -> Self {
        Self { parent:Arc::downgrade(parent), popup:Arc::downgrade(popup), bounds }
    }

    pub fn blocks_parent(&self, parent: &WindowState) -> bool {
        self.parent.upgrade().is_some_and(|p| std::ptr::eq(p.as_ref(),parent))
            && self.popup.upgrade().is_some_and(|p| !p.released.load(Ordering::Acquire))
    }

    pub fn refresh(&self) -> bool {
        let Some(popup) = self.popup.upgrade() else { return false; };
        if popup.gone.load(Ordering::Relaxed) { return !popup.released.load(Ordering::Acquire); }
        let parent = self.parent.upgrade().filter(|p| !p.gone.load(Ordering::Relaxed)
            && unsafe { IsWindowVisible(p.hwnd()) }.as_bool() && !unsafe { IsIconic(p.hwnd()) }.as_bool());
        let result = parent.as_ref().ok_or_else(|| "popup owner was hidden or retired".to_owned())
            .and_then(|p| self.move_with_parent(p,&popup));
        if let Err(error) = result {
            if parent.is_some() { eprintln!("popup · {error}"); }
            if let Some(k) = popup.popup { let _ = popup.to_render.send(ToRender::PopupClosed(k)); }
            retire(&popup);
        }
        // Keep the owner alive until the renderer releases its child's swapchain.
        !popup.released.load(Ordering::Acquire)
    }

    pub(super) fn move_with_parent(&self, parent: &WindowState, popup: &WindowState) -> Result<(), String> {
        let monitor = placement(parent,self.bounds)?;
        let changed = {
            let mut placement = popup.placement.lock().unwrap();
            let p = placement.as_mut().ok_or_else(|| "popup placement was retired".to_owned())?;
            let changed = p.monitor != monitor.rect || popup.scale() != monitor.scale;
            p.monitor = monitor.rect;
            changed
        };
        if changed {
            if popup.scale() != monitor.scale {
                popup.scale.store(monitor.scale.to_bits(),Ordering::Relaxed);
                let _ = popup.to_render.send(ToRender::Scale(popup.id,monitor.scale));
            }
            popup.output_changed(monitor);
            unsafe { reposition(popup); apply_input_region(popup); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_slides_inside_work_area_at_fractional_dpi_and_negative_origins() {
        let work = RECT { left:-1920,top:40,right:0,bottom:1040 };
        assert_eq!(slide(POINT{x:-100,y:1000},(260,150),1.25,work).unwrap(),
            RECT { left:-325,top:852,right:0,bottom:1040 });
        assert_eq!(slide(POINT{x:-2100,y:-20},(260,150),1.25,work).unwrap(),
            RECT { left:-1920,top:40,right:-1595,bottom:228 });
        assert_eq!(slide(POINT{x:-800,y:200},(260,150),1.25,work).unwrap(),
            RECT { left:-800,top:200,right:-475,bottom:388 });
    }
    #[test]
    fn oversized_popup_keeps_its_layout_and_invalid_geometry_is_rejected() {
        let work = RECT { left:0,top:40,right:800,bottom:600 };
        assert_eq!(slide(POINT{x:500,y:500},(1000,700),1.0,work).unwrap(),
            RECT { left:0,top:40,right:1000,bottom:740 });
        assert!(slide(POINT::default(),(10,10),f32::NAN,work).is_err());
        assert!(slide(POINT::default(),(i32::MAX,10),2.0,work).is_err());
        assert!(slide(POINT::default(),(10,10),1.0,RECT::default()).is_err());
    }
}
