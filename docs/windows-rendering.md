# Windows retained surfaces

Native Windows surfaces use a bounded retained canvas automatically only when
wgpu identifies the adapter as `Cpu` (software rendering). Physical and virtual
GPU defaults are unchanged. Configuration is read once when the GPU is created:

- `PLEAMAR_RETAINED_SURFACE=auto`, empty or unset: software adapters only.
- `PLEAMAR_RETAINED_SURFACE=1`: explicitly opt in on any Windows adapter.
- `PLEAMAR_RETAINED_SURFACE=0`: explicitly disable retention.
- `PLEAMAR_FULL_REPAINT=1`: override retention for reference measurements.
- `PLEAMAR_TIMING=1`: record the adapter, chosen policy, allocations and releases.

The swapchain's acquired texture cannot be assumed to hold the previous frame.
Known damage is cleared and painted into a separate canvas, then the complete
canvas is copied to the swapchain. Unknown damage repaints everything. Glass
uses its existing canvas; Linux and provider-lent frames are unchanged.
Requested retained BGRA texture capacity is limited to 32 MiB across the
renderer. This is extra capacity, not a bound on driver memory or a RAM saving.
Only localized damage starts an allocation. Closure, reconfiguration and
continuous full repaints retire it; budget exhaustion falls back to full paint.

## Evidence and limits

At `d64e4d23`, all six forced-retention/reference native image pairs matched
exactly for movement, opacity, blur, reopening and resize; twelve PNGs were
inspected. The fixture waits for visible glyphs because font discovery is
asynchronous. Current CI also runs without an override, checks the actual
adapter/policy/allocation trace and compares the automatic path's pixels.

The [same-run Marea comparison](https://github.com/SamuelHinestrosa/marea-plm/actions/runs/37613571984)
passed Windows/Linux at Marea `3af26181` / engine `d64e4d23`. It ran full paint,
forced retention and full paint on the same binary, scene and software runner.
All thirty PNGs, 282 samples and forty-two renderer reports were inspected.

| Median across measured intervals | Full before | Retained | Full after |
| --- | ---: | ---: | ---: |
| Collected Marea CPU (% of one core) | 19.060 | 6.718 | 18.904 |
| Collected Marea paint (ms/round) | 0.605 | 0.150 | 0.530 |
| Conversation paint (ms/round) | 231.395 | 228.270 | 232.450 |
| Conversation CPU (% of one core) | 384.831 | 385.654 | 386.232 |
| Conversation working set (MiB) | 179.734 | 180.409 | 177.166 |
| Conversation private commit (MiB) | 237.430 | 235.828 | 226.240 |

The idle CPU reduction supports the software-only default; conversation paint
improved only 1.35–1.80%. Private commit is not resident memory and this does not
show a RAM reduction. The adapter was Microsoft Basic Render Driver (Dx12).
The isolated scene used no live SDK, account, hardware services or physical
input. Physical-GPU performance, mixed-DPI and whole-product acceptance remain
separate. Visible fixtures run exclusively on disposable hosted CI.
