# Measuring pleamar against Quickshell

How the numbers in the README were taken, so they can be taken again the same
way. The point is to compare like with like: same machine, same monitor, both
freshly started, both animating.

## What to measure

```sh
# CPU, memory and threads of a process, over N seconds
medir() {
    pid=$1; sec=${2:-20}; label=${3:-process}
    hz=$(getconf CLK_TCK)
    c0=$(awk '{print $14+$15}' /proc/$pid/stat); t0=$(date +%s.%N)
    sleep "$sec"
    c1=$(awk '{print $14+$15}' /proc/$pid/stat); t1=$(date +%s.%N)
    rss=$(awk '/VmRSS/{print $2/1024}' /proc/$pid/status)
    pss=$(awk '/^Pss:/{s+=$2} END{print s/1024}' /proc/$pid/smaps_rollup)
    echo "$label · cpu $(echo "$c1 $c0 $hz $t1 $t0" | awk '{printf "%.2f", ($1-$2)/$3/($4-$5)*100}') % · rss $rss MB · pss $pss MB"
}
```

PSS is the honest memory figure: shared pages counted once, split between whoever
shares them.

## Frames and the cost of one

pleamar prints its own cycle when it exits, and with `PLEAMAR_CRONO=1` it breaks
down every frame:

```
cycle · 600 frames · mean 0.31 ms · p99 1.16 ms · max 1.63 ms
crono · hueco 16.04 · apuntar 0.10 · cerrar 0.21 · mandar+presentar 0.23 ms
```

"hueco" is waiting for the monitor: that is **sleep**, not CPU. What costs is the
rest, and it is the floor of wgpu on Vulkan: about 0.5 ms per frame whatever is
drawn (limitation P11).

For Quickshell, `QSG_RENDER_TIMING=1` makes Qt print one line per frame with
`polish`, `sync`, `render` and `swap`, plus `perWindowFrameDelta`. Count the
lines to get frames per second, average the numbers to get the cost of a frame.

## Restarting someone's bar without losing it

Save its environment and its command line **before** killing it, and put it back
the same way:

```sh
pid=$(pgrep -x quickshell)
tr '\0' ' ' < /proc/$pid/cmdline        # the command
tr '\0' '\n' < /proc/$pid/environ       # its variables, which may matter
```

A shell usually carries variables of its own (`MAREA_ROOT`, `MAREA_AVISOS`…). A
Python script that reads `/proc/<pid>/environ` into a dict and calls `Popen(env=…,
start_new_session=True)` puts it back exactly as it was.

## What to watch out for

- **Quickshell's numbers move** with how long it has been up and what has been
  opened. Freshly started: 5.75 % and 386 MB. After hours: 13.2 % and 188 MB.
  Always compare bars with the same uptime.
- **Opening a panel leaves things behind in QML**: CPU goes up and does not come
  back down. In pleamar it returns, because nothing is instantiated.
- **The two bars rarely hold the same things.** Say so when reporting: a panel
  with a chat inside is not the same panel.
- Measure with the mouse away from both, or the hover animations skew it.
