//! cpu1sec - Collect CPU usage data for munin every second
//!
// SPDX-License-Identifier:  GPL-3.0-only

#![warn(missing_docs)]

use anyhow::Result;
use log::{info, warn};
use munin_plugin::{Config, MuninPlugin};
use procfs::{CpuTime, KernelStats};
use simple_logger::SimpleLogger;
use std::{
    env,
    io::{BufWriter, Write},
    ops::Sub,
    time::{SystemTime, UNIX_EPOCH},
};

/// Stores CPU values (ticks), so we can easily put them in a vector,
/// substract them to know difference, ...
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct CpuStat {
    /// Number of CPU data is for. Will be [u32::MAX] for "total". If
    /// one really has so many CPU cores in their system: Sorry, lost,
    /// this plugin won't work (in detailed mode) then.
    cpu: u32,
    /// Epoch the data belongs to
    epoch: u64,
    /// Ticks spent in user mode
    user: u64,
    /// Ticks spent in user mode with low priority (nice)
    nice: u64,
    /// Ticks spent in system mode
    system: u64,
    /// Ticks spent in the idle state
    idle: u64,
    /// Ticks waiting for I/O to complete (unreliable)
    iowait: u64,
    /// Ticks servicing interrupts
    irq: u64,
    /// Ticks servicing softirqs
    softirq: u64,
    /// Ticks of stolen time.
    ///
    /// Stolen time is the time spent in other operating systems when
    /// running in a virtualized environment
    steal: u64,
    /// Ticks spent running a virtual CPU for guest operating systems
    /// under control of the linux kernel
    guest: u64,
    /// Ticks spent running a niced guest
    guest_nice: u64,
    /// Same as [CpuPlugin::cpudetail]
    cpudetail: bool,
}

/// Simple way of writing out the associated data
impl std::fmt::Display for CpuStat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // If you really have u32::max CPUs in your system then you
        // lost here. We take that as the field for "total".
        let cpu = if self.cpu == u32::MAX {
            if self.cpudetail {
                writeln!(f, "multigraph cpu1sec")?;
            }
            "total".to_string()
        } else {
            if self.cpudetail {
                writeln!(f, "multigraph cpu1sec.cpu{}", self.cpu)?;
            }
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
        writeln!(
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
            /// By default we assume we do graphs for "total"
            cpu: u32::max_value(),
            cpudetail: false,
            /// Data is for *right* *now*
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
            /// No sense substracting CPU number
            cpu: self.cpu,
            /// We always take the newer epoch
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
            /// Boolean value does not substract
            cpudetail: self.cpudetail,
        }
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
        cpudetail: false,
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
        cpudetail: true,
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
            cpudetail: false,
        },
        diff
    );
}

/// Take CpuTime and shove it into CpuStat
fn cpu_stat_to_value(cpu: u32, stat: CpuTime, cpudetail: bool) -> CpuStat {
    CpuStat {
        cpu,
        cpudetail,
        user: stat.user,
        nice: stat.nice,
        system: stat.system,
        idle: stat.idle,
        iowait: stat.iowait.unwrap_or(0),
        irq: stat.irq.unwrap_or(0),
        softirq: stat.softirq.unwrap_or(0),
        steal: stat.steal.unwrap_or(0),
        guest: stat.guest.unwrap_or(0),
        guest_nice: stat.guest_nice.unwrap_or(0),
        ..Default::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The struct for our plugin, so we can easily store some values over
/// the lifetime of our plugin.
struct CpuPlugin {
    /// Should we spit out data for detailed graphs for every CPU the system has, or just a total?
    /// The default will be determined:
    ///  * from the environment variable cpudetail, if set to 1, detailed graphs will be shown,
    ///  * anything else will be false, only total graph shown.
    cpudetail: bool,

    /// Store old CpuStat data to diff against
    old: Vec<CpuStat>,
}

impl Default for CpuPlugin {
    /// Set defaults
    fn default() -> Self {
        // Munin configuration for plugin goes via environment
        // variables
        let cpudetail = match env::var("cpudetail") {
            Ok(val) => val.eq(&"1"),
            Err(_) => false,
        };
        // Pre-fill the "old" data, so we always have something to
        // diff against in acquire
        let ks = KernelStats::new().expect("Could not read kernelstats");
        let mut old: Vec<CpuStat> = if cpudetail {
            ks.cpu_time
                .into_iter()
                .enumerate()
                .map(|(cpu, stat)| cpu_stat_to_value(cpu as u32, stat, cpudetail))
                .collect()
        } else {
            // If we do not want details, an empty vector is enough.
            // "Total" values get pushed to it next.
            vec![]
        };
        old.push(CpuStat {
            user: ks.total.user,
            nice: ks.total.nice,
            system: ks.total.system,
            idle: ks.total.idle,
            iowait: ks.total.iowait.unwrap_or(0),
            irq: ks.total.irq.unwrap_or(0),
            softirq: ks.total.softirq.unwrap_or(0),
            steal: ks.total.steal.unwrap_or(0),
            guest: ks.total.guest.unwrap_or(0),
            guest_nice: ks.total.guest_nice.unwrap_or(0),
            cpudetail,
            ..Default::default()
        });
        Self { cpudetail, old }
    }
}

impl CpuPlugin {
    /// Write out the detailed config per core/for totals, little helper for the config function
    fn write_details<W: Write>(&self, handle: &mut BufWriter<W>, cpu: &str) -> Result<()> {
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
        let uplimit = if cpu.eq("total") {
            procfs::CpuInfo::new()?.num_cores() * 100
        } else {
            100
        };
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
        writeln!(handle, "{cpu}_system.type GAUGE")?;
        writeln!(
            handle,
            "{cpu}_system.info CPU time spent by the kernel in system activities"
        )?;
        writeln!(handle, "{cpu}_user.label user")?;
        writeln!(handle, "{cpu}_user.draw STACK")?;
        writeln!(handle, "{cpu}_user.min 0")?;
        writeln!(handle, "{cpu}_user.type GAUGE")?;
        writeln!(
            handle,
            "{cpu}_user.info CPU time spent by normal programs and daemons"
        )?;
        writeln!(handle, "{cpu}_nice.label nice")?;
        writeln!(handle, "{cpu}_nice.draw STACK")?;
        writeln!(handle, "{cpu}_nice.min 0")?;
        writeln!(handle, "{cpu}_nice.type GAUGE")?;
        writeln!(
            handle,
            "{cpu}_nice.info CPU time spent by nice(1)d programs"
        )?;
        writeln!(handle, "{cpu}_idle.label idle")?;
        writeln!(handle, "{cpu}_idle.draw STACK")?;
        writeln!(handle, "{cpu}_idle.min 0")?;
        writeln!(handle, "{cpu}_idle.type GAUGE")?;
        writeln!(handle, "{cpu}_idle.info Idle CPU time")?;
        writeln!(handle, "{cpu}_iowait.label iowait")?;
        writeln!(handle, "{cpu}_iowait.draw STACK")?;
        writeln!(handle, "{cpu}_iowait.min 0")?;
        writeln!(handle, "{cpu}_iowait.type GAUGE")?;
        writeln!(handle, "{cpu}_iowait.info CPU time spent waiting for I/O operations to finish when there is nothing else to do.")?;
        writeln!(handle, "{cpu}_irq.label irq")?;
        writeln!(handle, "{cpu}_irq.draw STACK")?;
        writeln!(handle, "{cpu}_irq.min 0")?;
        writeln!(handle, "{cpu}_irq.type GAUGE")?;
        writeln!(handle, "{cpu}_irq.info CPU time spent handling interrupts")?;
        writeln!(handle, "{cpu}_softirq.label softirq")?;
        writeln!(handle, "{cpu}_softirq.draw STACK")?;
        writeln!(handle, "{cpu}_softirq.min 0")?;
        writeln!(handle, "{cpu}_softirq.type GAUGE")?;
        writeln!(
            handle,
            "{cpu}_softirq.info CPU time spent handling \"batched\" interrupts"
        )?;
        writeln!(handle, "{cpu}_steal.label steal")?;
        writeln!(handle, "{cpu}_steal.draw STACK")?;
        writeln!(handle, "{cpu}_steal.min 0")?;
        writeln!(handle, "{cpu}_steal.type GAUGE")?;
        writeln!(handle, "{cpu}_steal.info The time that a virtual CPU had runnable tasks, but the virtual CPU itself was not running")?;
        writeln!(handle, "{cpu}_guest.label guest")?;
        writeln!(handle, "{cpu}_guest.draw STACK")?;
        writeln!(handle, "{cpu}_guest.min 0")?;
        writeln!(handle, "{cpu}_guest.type GAUGE")?;
        writeln!(handle, "{cpu}_guest.info The time spent running a virtual CPU for guest operating systems under the control of the Linux kernel.")?;
        writeln!(handle, "{cpu}_guest_nice.label guest_nice")?;
        writeln!(handle, "{cpu}_guest_nice.draw STACK")?;
        writeln!(handle, "{cpu}_guest_nice.min 0")?;
        writeln!(handle, "{cpu}_guest_nice.type GAUGE")?;
        writeln!(handle, "{cpu}_guest_nice.info The time spent running a nice(1)d virtual CPU for guest operating systems under the control of the Linux kernel.")?;
        Ok(())
    }
}

impl MuninPlugin for CpuPlugin {
    fn config<W: Write>(&self, handle: &mut BufWriter<W>) -> Result<()> {
        if self.cpudetail {
            writeln!(handle, "multigraph cpu1sec")?;
        }
        self.write_details(handle, "total")?;
        if self.cpudetail {
            let numcores = procfs::CpuInfo::new()?.num_cores();
            for num in 0..numcores {
                let f = format!("cpu{num}");
                writeln!(handle, "multigraph cpu1sec.{f}")?;
                self.write_details(handle, &f)?;
            }
        }
        Ok(())
    }

    fn acquire<W: Write>(
        &mut self,
        handle: &mut BufWriter<W>,
        _config: &Config,
        epoch: u64,
    ) -> Result<()> {
        let cpudetail = self.cpudetail;

        let ks = KernelStats::new()?;
        let mut new: Vec<CpuStat> = if cpudetail {
            ks.cpu_time
                .into_iter()
                .enumerate()
                .map(|(cpu, stat)| cpu_stat_to_value(cpu as u32, stat, cpudetail))
                .collect()
        } else {
            vec![]
        };
        new.push(CpuStat {
            user: ks.total.user,
            nice: ks.total.nice,
            system: ks.total.system,
            idle: ks.total.idle,
            iowait: ks.total.iowait.unwrap_or(0),
            irq: ks.total.irq.unwrap_or(0),
            softirq: ks.total.softirq.unwrap_or(0),
            steal: ks.total.steal.unwrap_or(0),
            guest: ks.total.guest.unwrap_or(0),
            guest_nice: ks.total.guest_nice.unwrap_or(0),
            cpudetail,
            epoch,
            ..Default::default()
        });
        // Calculate the difference
        let diff: Vec<CpuStat> = self
            .old
            .iter()
            .zip(new.iter())
            .map(|i| (*i.1 - *i.0))
            .collect();

        for cpustat in diff {
            // Linebreak is added within the display of cpustat, so we do not need to do this
            write!(handle, "{cpustat}")?;
        }
        self.old = new;
        Ok(())
    }
}

fn main() -> Result<()> {
    SimpleLogger::new().init().unwrap();
    info!("cpu1sec started");

    // Set out config
    let mut config = Config::new(String::from("cpu1sec"));
    // Yes, we want to run as a daemon, gathering data once a second
    config.daemonize = true;
    // And our config output can be huge, especially if user wants a
    // detailed graph of every CPU
    config.cfgsize = procfs::CpuInfo::new()?.num_cores() * 3000;
    // Fetchsize 64k is arbitary, but better than default 8k.
    config.fetchsize = 65535;

    let mut cpu = CpuPlugin {
        ..Default::default()
    };

    // Get running
    cpu.start(config)?;
    Ok(())
}
