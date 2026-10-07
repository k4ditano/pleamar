//! A bounded canvas for Windows swapchains, whose acquired buffer contents are unknown.
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

/// Only the measured software path opts in automatically; hardware stays unchanged.
pub(super) fn enabled(adapter: wgpu::DeviceType, setting: Option<&str>, full_repaint: bool) -> bool {
    !full_repaint && match setting {
        Some("1") => true,
        None | Some("") | Some("auto") => adapter == wgpu::DeviceType::Cpu,
        _ => false,
    }
}

const MAX_PIXELS: u64 = 8_388_608; // 32 MiB across every retained BGRA canvas.
#[derive(Default)]
pub(super) struct Budget(Arc<AtomicU64>);
struct Reservation { used: Arc<AtomicU64>, pixels: u64 }
impl Drop for Reservation { fn drop(&mut self) { self.used.fetch_sub(self.pixels, Ordering::Relaxed); } }
impl Budget {
    fn reserve(&self, px: (u32, u32)) -> Option<Reservation> {
        let pixels = u64::from(px.0) * u64::from(px.1);
        if pixels == 0 { return None; }
        self.0.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |used| used.checked_add(pixels).filter(|total| *total <= MAX_PIXELS)).ok()?;
        Some(Reservation { used:self.0.clone(), pixels })
    }
}
pub(super) struct Canvas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub valid: bool,
    full_frames: u16,
    _room: Reservation,
}
impl Canvas {
    pub fn copy_to(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture) {
        encoder.copy_texture_to_texture(self.texture.as_image_copy(), target.as_image_copy(), self.texture.size());
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        if super::timing_enabled() { println!("timing · retained surface released {} pixels", self._room.pixels); }
    }
}

/// Conservative rounding includes antialiased edge pixels; invalid input repaints everything.
pub(super) fn region(rects: &[[f32; 4]], view: [f32; 4], scale: f32, px: (u32, u32)) -> Option<[u32; 4]> {
    if !scale.is_finite() || scale <= 0.0 || view.iter().any(|v| !v.is_finite()) || px.0 == 0 || px.1 == 0 {
        return None;
    }
    let mut union = [px.0, px.1, 0, 0];
    for rect in rects {
        if rect.iter().any(|v| !v.is_finite()) || rect[2] < rect[0] || rect[3] < rect[1] { return None; }
        if rect[0] >= view[2] || rect[2] <= view[0] || rect[1] >= view[3] || rect[3] <= view[1] { continue; }
        let bounds = [((rect[0]-view[0])*scale).floor().clamp(0.0,px.0 as f32) as u32,
            ((rect[1]-view[1])*scale).floor().clamp(0.0,px.1 as f32) as u32,
            ((rect[2]-view[0])*scale).ceil().clamp(0.0,px.0 as f32) as u32,
            ((rect[3]-view[1])*scale).ceil().clamp(0.0,px.1 as f32) as u32];
        union = [union[0].min(bounds[0]),union[1].min(bounds[1]),union[2].max(bounds[2]),union[3].max(bounds[3])];
    }
    Some(if union[2] <= union[0] || union[3] <= union[1] { [0;4] }
        else { [union[0],union[1],union[2]-union[0],union[3]-union[1]] })
}

pub(super) fn effect_damage(draw: &super::DrawList, rects: &[[f32;4]]) -> Vec<[f32;4]> {
    let mut damage = rects.to_vec();
    for (span, _) in &draw.offscreen_groups {
        // The blend after a group's contents includes its blur/glow margin.
        // A changed child can affect that margin without changing the blend's parameters.
        let offset = span.end as usize * super::PER_ELEMENT + 4;
        if let Some(bounds) = draw.elements.get(offset..offset+4) {
            let bounds = [bounds[0],bounds[1],bounds[2],bounds[3]];
            if rects.iter().any(|r| r[0]<bounds[2] && r[2]>bounds[0] && r[1]<bounds[3] && r[3]>bounds[1]) {
                damage.push(bounds);
            }
        }
    }
    damage
}

pub(super) fn prepare(canvas: &mut Option<Canvas>, budget: &Budget, device: &wgpu::Device,
    format: wgpu::TextureFormat, px: (u32,u32), view: [f32;4], scale: f32,
    damage: Option<&[[f32;4]]>, enabled: bool) {
    if !enabled || !matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        | wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb) { *canvas = None; return; }
    let localized = damage.and_then(|rects| region(rects,view,scale,px))
        .is_some_and(|r| r[2]>0 && r[3]>0 && u64::from(r[2])*u64::from(r[3])*4 < u64::from(px.0)*u64::from(px.1)*3);
    if let Some(saved) = canvas {
        saved.full_frames = if localized { 0 } else { saved.full_frames.saturating_add(1) };
        // Continuous whole-surface animations gain nothing from a retained copy.
        if saved.full_frames >= 120 { *canvas = None; }
    }
    if !localized || canvas.is_some() { return; }
    let Some(room) = budget.reserve(px) else { return; };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained Windows surface"),
        size: wgpu::Extent3d { width:px.0,height:px.1,depth_or_array_layers:1 },
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,
        usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[],
    });
    let view = texture.create_view(&Default::default());
    if super::timing_enabled() { println!("timing · retained surface allocated {}×{}", px.0, px.1); }
    *canvas = Some(Canvas { texture,view,valid:false,full_frames:0,_room:room });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn automatic_retention_only_selects_software_and_respects_overrides() {
        use wgpu::DeviceType::*;
        for adapter in [Cpu,DiscreteGpu,IntegratedGpu,VirtualGpu,Other] {
            for setting in [None,Some(""),Some("auto")] {
                assert_eq!(enabled(adapter,setting,false),adapter==Cpu);
            }
            assert!(enabled(adapter,Some("1"),false));
            for setting in [Some("0"),Some("invalid")] { assert!(!enabled(adapter,setting,false)); }
            for setting in [None,Some("auto"),Some("1"),Some("0")] { assert!(!enabled(adapter,setting,true)); }
        }
    }
    #[test] fn damage_rounds_outwards_clamps_and_unites_different_positions() {
        let view = [-100.0,20.0,700.0,620.0];
        assert_eq!(region(&[[-99.9,20.1,-89.9,30.1]],view,1.25,(1000,750)),Some([0,0,13,13]));
        assert_eq!(region(&[[-200.0,0.0,-90.0,30.0],[690.0,610.0,900.0,800.0]],view,1.25,(1000,750)),Some([0,0,1000,750]));
        assert_eq!(region(&[[800.0,700.0,900.0,800.0]],view,1.25,(1000,750)),Some([0;4]));
        assert_eq!(region(&[],view,1.25,(1000,750)),Some([0;4]));
        assert_eq!(region(&[[0.0,0.0,f32::NAN,3.0]],view,1.25,(1000,750)),None);
        assert_eq!(region(&[[4.0,4.0,3.0,5.0]],view,1.25,(1000,750)),None);
        assert_eq!(region(&[],view,0.0,(1000,750)),None);
    }
    #[test] fn shared_budget_rejects_oversized_canvases_and_releases_exactly() {
        let budget=Budget::default();
        assert!(budget.reserve((0,100)).is_none());
        assert!(budget.reserve((u32::MAX,u32::MAX)).is_none());
        let first=budget.reserve((2048,2048)).unwrap();
        let second=budget.reserve((2048,2048)).unwrap();
        assert!(budget.reserve((1,1)).is_none());
        drop(first);
        let third=budget.reserve((2048,2048)).unwrap();
        drop(second);drop(third);
        assert_eq!(budget.0.load(Ordering::Relaxed),0);
    }
    #[test] fn child_damage_includes_the_groups_effect_margin() {
        let mut draw=super::super::DrawList::default();
        draw.elements=vec![0.0;super::super::PER_ELEMENT*2];
        draw.elements[super::super::PER_ELEMENT+4..super::super::PER_ELEMENT+8].copy_from_slice(&[10.0,20.0,100.0,110.0]);
        draw.offscreen_groups.push((0..1,0));
        let changed=[30.0,40.0,50.0,60.0];
        assert_eq!(effect_damage(&draw,&[changed]),vec![changed,[10.0,20.0,100.0,110.0]]);
        let outside=[300.0,300.0,350.0,350.0];
        assert_eq!(effect_damage(&draw,&[outside]),vec![outside]);
    }
}
