# cpu1sec

A munin plugin to collect CPU statistic data once every second.

## How
The plugin uses data available in procfs to display CPU
idle/iowait/user/system/... data. Statistics are read once per second
and written to a cachefile, whenever munin asks for the data, the
content of the cachefile is send.

## Usage
Compile (or load a released binary, if one is there) and put the
binary somewhere. Then link it into the munin plugins dir.

When first called without arguments, cpu1sec will spawn itself into the
background to gather data. This can also be triggered by calling it
with the `acquire` parameter.

## Detailed graphs
Per default this plugin will only produce a "total" graph, similar to
what the default cpu plugin from munin does (though in much higher
resolution, obviously).

The plugin can produce the graphs per CPU/core on the system. Then the
view "total" will be the front, when you click it, you get a detail
graph of every CPU core on the system.

To activate this, set the environment variable cpudetail to 1, say in
`/etc/munin/plugin-conf.d/cpu1sec.conf` put
```
[cpu1sec]
env.cpudetail=1
```

### Filesize
Note that every graph has 10 datasets, and each dataset uses ~11MB on
disk. If you have many cores, enabling cpudetail may easily run you
out of disk space!

## Local build
Use cargo build as usual. Note that the release build contains much
less logging code than the debug build, so if you want to find out,
why something does not work as planned, ensure to use a debug build
(`cargo build` instead of `cargo build --release`).

## Musl
Note that I build using musl, as I want fully static binaries.
"Normal" rust link against libc, and that may carry symbols that
aren't available everywhere (older versions). If you do not have that
requirement, not using musl will be fine.

## Packages
Note that they do not live up to distribution quality, but they do
work for an easy install.

### Debian package
A minimal Debian package can be build using `cargo deb`, provided that
you installed this feature (`cargo install cargo-deb`).

### RPM package
A minimal package for RPM based systems can be build using `cargo
generate-rpm`, provided that you installed this feature (`cargo install cargo-generate-rpm`)
