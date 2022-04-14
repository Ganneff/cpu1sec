//! cpu1sec - Collect CPU usage data for munin every second
//!
// SPDX-License-Identifier:  GPL-3.0-only

#![warn(missing_docs)]

use daemonize::Daemonize;
use fs2::FileExt;
use log::{debug, error, info, trace, warn};
use procfs::{CpuTime, KernelStats};
use simple_logger::SimpleLogger;
use spin_sleep::LoopHelper;
use std::{
    env,
    error::Error,
    fs::{rename, File, OpenOptions},
    io::{self, BufWriter, Write},
    ops::Sub,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

fn write_details<W: Write>(handle: &mut BufWriter<W>, cpu: &str) -> Result<(), Box<dyn Error>> {
    let uplimit = procfs::CpuInfo::new()?.num_cores() * 100;
    writeln!(handle, "graph_title CPU usage {cpu} (1sec)")?;
    writeln!(handle, "graph_category system")?;
    writeln!(handle, "update_rate 1",)?;
    writeln!(
        handle,
        "graph_data_size custom 1d, 1s for 1d, 5s for 2d, 10s for 7d, 1m for 1t, 5m for 1y",
    )?;
    writeln!(
        handle,
        "graph_order system user nice idle iowait irq softirq"
    )?;
    writeln!(
        handle,
        "graph_args --base 1000 -r --lower-limit 0 --upper-limit {}",
        uplimit
    )?;
    writeln!(handle, "graph_vlabel %")?;
    writeln!(handle, "graph_scale no")?;
    writeln!(handle, "graph_info This graph shows how CPU time is spent.")?;

    writeln!(handle, "{cpu}_system.label system")?;
    writeln!(handle, "{cpu}_system.draw AREA")?;
    writeln!(handle, "{cpu}_system.min 0")?;
    writeln!(handle, "{cpu}_system.type DERIVE")?;
    writeln!(
        handle,
        "{cpu}_system.info CPU time spent by the kernel in system activities"
    )?;
    writeln!(handle, "{cpu}_user.label user")?;
    writeln!(handle, "{cpu}_user.draw STACK")?;
    writeln!(handle, "{cpu}_user.min 0")?;
    writeln!(handle, "{cpu}_user.type DERIVE")?;
    writeln!(
        handle,
        "{cpu}_user.info CPU time spent by normal programs and daemons"
    )?;
    writeln!(handle, "{cpu}_nice.label nice")?;
    writeln!(handle, "{cpu}_nice.draw STACK")?;
    writeln!(handle, "{cpu}_nice.min 0")?;
    writeln!(handle, "{cpu}_nice.type DERIVE")?;
    writeln!(
        handle,
        "{cpu}_nice.info CPU time spent by nice(1)d programs"
    )?;
    writeln!(handle, "{cpu}_idle.label idle")?;
    writeln!(handle, "{cpu}_idle.draw STACK")?;
    writeln!(handle, "{cpu}_idle.min 0")?;
    writeln!(handle, "{cpu}_idle.type DERIVE")?;
    writeln!(handle, "{cpu}_idle.info Idle CPU time")?;
    writeln!(handle, "{cpu}_iowait.label iowait")?;
    writeln!(handle, "{cpu}_iowait.draw STACK")?;
    writeln!(handle, "{cpu}_iowait.min 0")?;
    writeln!(handle, "{cpu}_iowait.type DERIVE")?;
    writeln!(handle, "{cpu}_iowait.info CPU time spent waiting for I/O operations to finish when there is nothing else to do.")?;
    writeln!(handle, "{cpu}_irq.label irq")?;
    writeln!(handle, "{cpu}_irq.draw STACK")?;
    writeln!(handle, "{cpu}_irq.min 0")?;
    writeln!(handle, "{cpu}_irq.type DERIVE")?;
    writeln!(handle, "{cpu}_irq.info CPU time spent handling interrupts")?;
    writeln!(handle, "{cpu}_softirq.label softirq")?;
    writeln!(handle, "{cpu}_softirq.draw STACK")?;
    writeln!(handle, "{cpu}_softirq.min 0")?;
    writeln!(handle, "{cpu}_softirq.type DERIVE")?;
    writeln!(
        handle,
        "{cpu}_softirq.info CPU time spent handling \"batched\" interrupts"
    )?;
    writeln!(handle, "{cpu}_steal.label steal")?;
    writeln!(handle, "{cpu}_steal.draw STACK")?;
    writeln!(handle, "{cpu}_steal.min 0")?;
    writeln!(handle, "{cpu}_steal.type DERIVE")?;
    writeln!(handle, "{cpu}_steal.info The time that a virtual CPU had runnable tasks, but the virtual CPU itself was not running")?;
    writeln!(handle, "{cpu}_guest.label guest")?;
    writeln!(handle, "{cpu}_guest.draw STACK")?;
    writeln!(handle, "{cpu}_guest.min 0")?;
    writeln!(handle, "{cpu}_guest.type DERIVE")?;
    writeln!(handle, "{cpu}_guest.info The time spent running a virtual CPU for guest operating systems under the control of the Linux kernel.")?;
    Ok(())
}

/// Print out munin config data
///
/// Will print out config data per host as listed from [get_hosts],
/// preparing for multiple graphs, one summary and 3 detail ones.
fn config() -> Result<(), Box<dyn Error>> {
    // We want to write a large amount to stdout, take and lock it
    let stdout = io::stdout();
    let numcores = procfs::CpuInfo::new()?.num_cores();
    let bufsize = numcores * 3000;
    let mut handle = BufWriter::with_capacity(bufsize, stdout.lock());

    writeln!(handle, "multigraph cpu1sec")?;
    write_details(&mut handle, "total")?;
    for num in 0..numcores {
        let f = format!("cpu{num}");
        writeln!(handle, "multigraph cpu1sec.{f}")?;
        write_details(&mut handle, &f)?;
    }
    // And flush it, so it can also deal with possible errors
    handle.flush()?;

    Ok(())
}

/// Gather the data from the system.
///
/// Daemonize into background and then run a loop forever, that
/// fetches data once a second and appends it to files in the given cachepath.
///
/// We read the values from the statistic files and parse them to a
/// [u64], that ought to be big enough to not overflow.
fn acquire(cachefile: &Path, pidfile: &Path) -> Result<(), Box<dyn Error>> {
    trace!("Going to daemonize");

    // We want to run as daemon, so prepare
    let daemonize = Daemonize::new()
        .pid_file(pidfile)
        .chown_pid_file(true)
        .working_directory("/tmp");

    // And off into the background we go
    daemonize.start()?;

    // The loop helper makes it easy to repeat a loop once a second
    let mut loop_helper = LoopHelper::builder().build_with_target_rate(1); // Only once a second

    // Buffer size is count of cpu times 400, that is enough
    let bufsize = procfs::CpuInfo::new()?.num_cores() * 400;

    debug!("{cachefile:#?}, buffersize: {bufsize}");
    // Fetch data once already, so old is not empty when we go into loop and calculate first diff.
    // First diff MAY end up pretty small, but that doesn't matter
    let mut old: Vec<CpuStat> = KernelStats::new()
        .unwrap()
        .cpu_time
        .into_iter()
        .enumerate()
        .map(|(cpu, stat)| cpu_stat_to_value(cpu as u32, stat))
        .collect();
    // We want CPU total too, so add it
    let mut ks = KernelStats::new()?.total;
    old.push(CpuStat {
        user: ks.user,
        nice: ks.nice,
        system: ks.system,
        idle: ks.idle,
        iowait: ks.iowait.expect("Dang"),
        irq: ks.irq.expect("Dang"),
        softirq: ks.softirq.expect("Dang"),
        steal: ks.steal.expect("Dang"),
        guest: ks.guest.expect("Dang"),
        guest_nice: ks.guest_nice.expect("Dang"),
        ..Default::default()
    });

    // We run forever
    loop {
        // Let loop helper prepare
        loop_helper.loop_start();

        // Get current CPU stat data
        let mut new: Vec<CpuStat> = KernelStats::new()?
            .cpu_time
            .into_iter()
            .enumerate()
            .map(|(cpu, stat)| cpu_stat_to_value(cpu as u32, stat))
            .collect();
        // And add the total one
        ks = KernelStats::new()?.total;
        new.push(CpuStat {
            user: ks.user,
            nice: ks.nice,
            system: ks.system,
            idle: ks.idle,
            iowait: ks.iowait.expect("Dang"),
            irq: ks.irq.expect("Dang"),
            softirq: ks.softirq.expect("Dang"),
            steal: ks.steal.expect("Dang"),
            guest: ks.guest.expect("Dang"),
            guest_nice: ks.guest_nice.expect("Dang"),
            ..Default::default()
        });
        // Calculate the difference
        let diff: Vec<CpuStat> = old
            .iter()
            .zip(new.iter())
            .map(|i| (i.1.clone() - i.0.clone()))
            .collect();

        // Want to ensure the cache file is closed, before we sleep
        {
            // Write out data to cache file
            let mut cachefd = BufWriter::with_capacity(
                bufsize,
                OpenOptions::new()
                    .create(true) // If not there, create
                    .write(true) // We want to write
                    .append(true) // We want to append
                    .open(&cachefile)?,
            );

            for cpustat in diff {
                writeln!(cachefd, "{cpustat}")?;
            }
        }
        // And save value for next round
        old = new;

        // Sleep for the rest of the second
        loop_helper.loop_sleep();
    }
}

/// Hand out the collected cpu data
///
/// Basically a "mv file tmpfile && cat tmpfile && rm tmpfile",
/// as the file is already in proper format
fn fetch(cache: &Path) -> Result<(), Box<dyn Error>> {
    // We need a temporary file
    let fetchpath =
        NamedTempFile::new_in(cache.parent().expect("Could not find useful temp path"))?;
    debug!("Fetchcache: {:?}, Cache: {:?}", fetchpath, cache);
    // Rename the cache file, to ensure that acquire doesn't add data
    // between us outputting data and deleting the file
    rename(&cache, &fetchpath)?;
    // We want to write possibly large amount to stdout, take and lock it
    let stdout = io::stdout();
    let mut handle = BufWriter::with_capacity(65536, stdout.lock());
    // Want to read the tempfile now
    let mut fetchfile = std::fs::File::open(&fetchpath)?;
    // And ask io::copy to just take it all and shove it into stdout
    io::copy(&mut fetchfile, &mut handle)?;
    handle.flush()?;
    Ok(())
}

/// Store CPU values
#[derive(Debug, Clone, PartialEq)]
struct CpuStat {
    cpu: u32,
    epoch: u64,
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
    guest: u64,
    guest_nice: u64,
}

/// Simple way of writing out the associated data
impl std::fmt::Display for CpuStat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // If you really have u32::max CPUs in your system then you
        // lost here. We take that as the field for "total".
        let cpu = if self.cpu == u32::max_value() {
            "total".to_string()
        } else {
            format!("cpu{}", self.cpu)
        };

        writeln!(f, "{}_user.value {}:{}", cpu, self.epoch, self.user)?;
        writeln!(f, "{}_nice.value {}:{}", cpu, self.epoch, self.nice)?;
        writeln!(f, "{}_system.value {}:{}", cpu, self.epoch, self.system)?;
        writeln!(f, "{}_idle.value {}:{}", cpu, self.epoch, self.idle)?;
        writeln!(f, "{}_iowait.value {}:{}", cpu, self.epoch, self.iowait)?;
        writeln!(f, "{}_irq.value {}:{}", cpu, self.epoch, self.irq)?;
        writeln!(f, "{}_softirq.value {}:{}", cpu, self.epoch, self.softirq)?;
        writeln!(f, "{}_steal.value {}:{}", cpu, self.epoch, self.steal)?;
        writeln!(f, "{}_guest.value {}:{}", cpu, self.epoch, self.guest)?;
        write!(
            f,
            "{}_guest_nice.value {}:{}",
            cpu, self.epoch, self.guest_nice
        )?;
        Ok(())
    }
}

/// Defaults, mainly setting the epoch to the second of "creation" of
/// this dataset
impl Default for CpuStat {
    fn default() -> Self {
        CpuStat {
            cpu: u32::max_value(),
            epoch: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Couldn't get epoch")
                .as_secs(),
            user: 0,
            nice: 0,
            system: 0,
            idle: 0,
            iowait: 0,
            irq: 0,
            softirq: 0,
            steal: 0,
            guest: 0,
            guest_nice: 0,
        }
    }
}

/// For diffing, we want to be able to substract CpuStats
impl Sub for CpuStat {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self {
            cpu: self.cpu,
            epoch: other.epoch,
            user: self.user - other.user,
            nice: self.nice - other.nice,
            system: self.system - other.system,
            idle: self.idle - other.idle,
            iowait: self.iowait - other.iowait,
            irq: self.irq - other.irq,
            softirq: self.softirq - other.softirq,
            steal: self.steal - other.steal,
            guest: self.guest - other.guest,
            guest_nice: self.guest_nice - other.guest_nice,
        }
    }
}

/// Take CpuTime and shove it into CpuStat
fn cpu_stat_to_value(cpu: u32, stat: CpuTime) -> CpuStat {
    CpuStat {
        cpu,
        user: stat.user,
        nice: stat.nice,
        system: stat.system,
        idle: stat.idle,
        iowait: stat.iowait.unwrap(),
        irq: stat.irq.unwrap(),
        softirq: stat.softirq.unwrap(),
        steal: stat.steal.unwrap(),
        guest: stat.guest.unwrap(),
        guest_nice: stat.guest_nice.unwrap(),
        ..Default::default()
    }
}

#[test]
fn test_sub() {
    let one = CpuStat {
        cpu: 2,
        epoch: 0,
        user: 42,
        nice: 42,
        system: 42,
        idle: 42,
        iowait: 42,
        irq: 42,
        softirq: 42,
        steal: 42,
        guest: 42,
        guest_nice: 42,
    };

    let two = CpuStat {
        cpu: 1,
        epoch: 1,
        user: 21,
        nice: 21,
        system: 21,
        idle: 21,
        iowait: 21,
        irq: 21,
        softirq: 21,
        steal: 21,
        guest: 21,
        guest_nice: 21,
    };
    let diff = one - two;
    assert_eq!(
        CpuStat {
            cpu: 2,
            epoch: 1,
            user: 21,
            nice: 21,
            system: 21,
            idle: 21,
            iowait: 21,
            irq: 21,
            softirq: 21,
            steal: 21,
            guest: 21,
            guest_nice: 21,
        },
        diff
    );
}

/// Manage it all.
///
/// Note that, while we do have extensive logging statements all over
/// the code, we use the crates feature to **not** compile in levels
/// we do not want. So in devel/debug builds, we have all levels
/// including trace! available, release build will only show warn! and
/// error! logs (tiny amount).
fn main() {
    SimpleLogger::new().init().unwrap();
    info!("cpu1sec started");
    // Store arguments for later use
    let args: Vec<String> = env::args().collect();

    // Where is our plugin state directory?
    let plugstate = env::var("MUNIN_PLUGSTATE").unwrap_or_else(|_| "/tmp".to_owned());
    debug!("Plugin State: {:#?}", plugstate);
    // Put our cache file there
    let cache = Path::new(&plugstate).join("munin.cpu1sec.value");
    debug!("Cache: {:?}", cache);
    // Our pid is stored here - we also use it to detect if the daemon
    // part is running, and if not, to start it when called to fetch
    // data.
    let pidfile = Path::new(&plugstate).join("munin.cpu1sec.pid");
    debug!("PIDfile: {:?}", pidfile);

    // Does the master support dirtyconfig?
    let dirtyconfig = match env::var("MUNIN_CAP_DIRTYCONFIG") {
        Ok(val) => val.eq(&"1"),
        Err(_) => false,
    };
    debug!("Dirtyconfig is: {:?}", dirtyconfig);

    // Now go over our other args and see what we are supposed to do
    match args.len() {
        // no arguments passed, print data
        1 => {
            trace!("No argument, assuming fetch");
            // Before we fetch we should ensure that we have a data
            // gatherer running. It locks the pidfile, so lets see if
            // it's locked or we can have it.
            let lockfile = !Path::exists(&pidfile) || {
                let lockedfile = File::open(&pidfile).expect("Could not open pidfile");
                lockedfile.try_lock_exclusive().is_ok()
            };

            // If we could lock, it appears that acquire isn't running. Start it.
            if lockfile {
                debug!("Could lock the pidfile, will spawn acquire now");
                Command::new(&args[0])
                    .arg("acquire".to_owned())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("failed to execute acquire");
                debug!("Spawned, sleep for 1s, then continue");
                // Now we wait one second before going on, so the
                // newly spawned process had a chance to generate us
                // some data
                thread::sleep(Duration::from_secs(1));
                // }
            }
            // And now we can hand out the cached data
            if let Err(e) = fetch(&cache) {
                error!("Could not fetch data: {}", e);
                std::process::exit(6);
            }
        }

        // one argument passed, check it and do something
        2 => match args[1].as_str() {
            "config" => {
                trace!("Called to hand out config");
                config().expect("Could not write out config");
                // If munin supports the dirtyconfig feature, we can hand out the data
                if dirtyconfig {
                    if let Err(e) = fetch(&cache) {
                        error!("Could not fetch data: {}", e);
                        std::process::exit(6);
                    }
                };
            }
            "acquire" => {
                trace!("Called to gather data");
                // Only will ever process anything after this line, if
                // one process has our pidfile already locked, ie. if
                // another acquire is running. (Or if we can not
                // daemonize for another reason).
                if let Err(e) = acquire(&cache, &pidfile) {
                    error!("Error: {}", e);
                    std::process::exit(5);
                };
            }
            _ => {
                error!("Unknown command {}", args[1]);
                std::process::exit(3);
            }
        },
        // all the other cases
        _ => {
            error!("Unknown number of arguments");
            std::process::exit(4);
        }
    }
    info!("All done");
}
