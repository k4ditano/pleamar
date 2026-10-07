# Native window capture transport

Windows window providers can receive `ToNest::WindowsGpu` after the renderer
has selected its real D3D12 device. `SharedDevice` creates bounded BGRA textures
on that device. The capture producer opens them on D3D11 using the same adapter
LUID; this is not cross-adapter sharing.

The Windows WM provider uses two shared images per active capture. It copies a
WGC frame into an available image, signals a D3D11 fence and publishes the image
only after that fence completes. A busy producer is polled without blocking the
GPU queue, with a five-second timeout. The renderer copies the published image
into its window texture array. There is no CPU pixel vector, staging readback
or CPU-to-GPU upload on this transport. There are still two GPU copies.

Each published image is immutable. The provider may overwrite it only while it
owns the sole strong `Arc`. The renderer retains its last displayed image for
texture-array growth, and retains every submitted copy until GPU completion.
The queue also owns a completion guard so exiting the render loop cannot release
a producer's image prematurely. The imported wgpu texture belongs to that image;
closing the final owner retires the import without requiring another repaint.

The existing CPU transport remains available when D3D12 sharing or D3D11 fences
are unsupported, when opening the shared image fails, or when the renderer
rejects it. Such failures are reported in the process log. Default Luau support
is unchanged. Linux continues to use its existing dmabuf transport.

For driver diagnostics or an A/B comparison with the same executable, set
`$env:PLEAMAR_WM_CAPTURE_CPU = '1'` before launching `pleamar-wm`. This explicitly
retains the CPU transport; remove the environment variable to use automatic
GPU negotiation again. The process log reports which transport was selected.

The provider's aggregate 16,777,216-pixel limit describes source images, not
total graphics memory. A capture also owns WGC buffers and up to two shared
images; the renderer's texture array includes padding and separate storage.
This does not establish a total GPU memory cap or support for rain, snow,
redirected input, independent seats, or a complete Windows compositor.

## Validation

### Scene output mapping

The Windows renderer also sends `ToNest::WindowsScreens`: pairs of logical
main-surface copy indices and their actual native output names. It republishes
changed mappings, including an empty mapping and the first mapping after a
scene reload. Named surfaces and popups do not participate. Providers must not
infer these indices from the order returned by a native monitor enumeration.

The companion WM uses this mapping for `--preview-monitor all` and keeps the
single-source preview option, including previewing that source on a different
display. Source dimensions use each window's current monitor DPI; a retained
frame can update logical geometry through `PieceContent::Kept` without copying
its pixels again. Removing output zero does not renumber surviving copy one.
Unit tests cover these mappings and DPI conversions. Native mixed-DPI movement,
hotplug and the new multi-output example remain unverified; the following
capture measurements predate this mapping change.

Decorated pleamar windows retain their launch placement identity while tracking
their current native output separately. A move publishes `ToRender::WindowsOutput`
with the current name and refresh rate; the renderer updates the main copy's
name and recalculates frame pacing. Refresh changes on an unchanged monitor
use the existing topology reconciliation, without waking the renderer when
the metadata is unchanged. A popup inherits its parent's current output.
Desktop-relative positions use the current monitor and the client corner,
excluding the native title bar and border. Hidden-window API regressions cover
metadata publication and client coordinates; they do not establish physical
mixed-DPI migration or visual correctness on multiple monitors.

On 2026-10-07 the default-Luau Windows library suite passed 216 tests, with
42 opt-in helpers skipped. The initial follow-up run found a topology fixture
that did not initialize the new output cache like production creation; its
initialization was corrected and its original removal assertion still passes.
Both hidden-window metadata/client-position regressions passed. The real
multi-monitor and frame-pacing walkthrough remains pending.

### Native preview texture retirement (2026-10-07)

After all preview demand ends and the provider has retired every picture,
the Windows renderer replaces its peak window texture array with a 1x1x1
placeholder. A page switch with outstanding demand retains the array. Queued
shared copies prevent retirement; submitted work retains its resources until
the GPU finishes, and any CPU uploads are submitted before replacement.
Reopening allocates room for the new pictures and rebuilds their binding.

The default-Luau release library suite passes 218 ordinary tests. The separate
`gpu::windows_memory_tests::retired_preview_capacity_reopens_with_fresh_pixels`
passes on the native D3D12 adapter: four close/reopen cycles, sixteen exact BGRA
layer images, pending shared-copy rejection, transparent replacement and fresh
pixels after reopening. The fixture's texel capacity falls from 1280x768x4
(15 MiB) to one BGRA pixel. This is resource capacity, not a measurement of
driver allocation, process RAM or whole-Marea VRAM. The existing twelve-copy
D3D11-to-D3D12 pixel/lifetime regression also passes. Neither test opens a
window or sends desktop input; actual overview close/reopen acceptance remains
pending a non-primary test display. CI explicitly runs both GPU regressions.

Windows refresh values 0 and 1 mean an unspecified hardware default, according
to Microsoft's [DEVMODE contract](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-devmodea).
They now stay unknown in output metadata. The shared frame clock uses a 60 Hz
fallback for an unknown refresh instead of treating it as 1 mHz and scheduling
a 1000-second mailbox wait. Tests cover unknown and negative values, fractional
59.94 Hz, 144 Hz and scene rate limits, including an overflowing integer limit.
Valid reported rates retain their existing pacing. This does not establish
mixed-refresh presentation quality or a measured 60 Hz hardware refresh.

### Capture transport

Windows x64/MSVC with default Luau was exercised on 2026-10-06. The engine's
202 ordinary tests and the WM's 18 ordinary tests pass. The opt-in native
`windows_texture::tests::shared_capture_reuses_native_pixels_without_cpu_upload`
passes 12 exact pixel comparisons, image reuse, two sizes, retired ownership
and rejection of a different logical wgpu device. An initial
`OpenSharedResource1` failure exposed the required render-target binding flag;
the corrected final build passes.

An owned passive scene on non-primary DISPLAY2 at 125% DPI passed source
updates, resize, Luau, watched reload, closure of each source and clean exit.
The actual Marea overview scene also passed pages 0/1/0/1 with six owned
windows, hidden-page capture demand, fresh pixels on return and Luau closure.
The foreground stayed unchanged; no physical mouse or keyboard input was sent.

The same final executable was measured using two 1200x700 sources, with one
resized to 1500x800, and a 16-second repaint sample:

| Transport | Process CPU seconds | Resident memory after sample | Private memory after sample |
| --- | ---: | ---: | ---: |
| Shared GPU images | 0.812 | 431.6 MiB | 206.4 MiB |
| Forced CPU readback | 1.938 | 186.6 MiB | 240.3 MiB |

The shared-image run had an unexplained resident-memory spike. A 32-second
repeat using `PROCESS_MEMORY_COUNTERS_EX2` measured 153.5 MiB
resident / 207.8 MiB private commit for shared images, and
186.8 / 240.8 MiB for CPU readback.
CPU time was 1.516 versus 3.641 seconds. The
spike did not recur, but its cause has not been established. Both comparisons
show lower CPU time and private commit; a reliable RSS reduction is not claimed.
The evidence retains the outlier and the repeat's two-second memory samples.

These are process measurements for this short capture workload, not whole-Marea
performance, UI frame time, sustained load or total VRAM. Closing the sources
released capture handles; that measured revision retained its texture-array
capacity. The later retirement change above has separate component evidence.
See the [commands, source hashes and structured evidence](windows-shared-capture-validation.json),
[shared-image scene](windows-shared-capture/shared-scene-reloaded.png),
[CPU comparison](windows-shared-capture/cpu-scene-reloaded.png) and
[Marea overview page](windows-shared-capture/marea-page-1.png).

The interop follows Microsoft's [shared heap contract](https://learn.microsoft.com/en-us/windows/win32/direct3d12/shared-heaps)
and [simultaneous-access state promotion and decay](https://learn.microsoft.com/en-us/windows/win32/direct3d12/using-resource-barriers-to-synchronize-resource-states-in-direct3d-12).
The HAL import starts in `COMMON` (`TextureUses::PRESENT` in wgpu's D3D12 mapping);
the first copy transitions to `COPY_SOURCE`. The image decays to `COMMON` after
submission and can implicitly promote on a subsequent copy.
