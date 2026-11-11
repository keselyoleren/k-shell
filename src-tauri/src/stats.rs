//! Server stats for the monitoring panel: host info, CPU, memory, network, disks, processes.
//!
//! Nothing is installed on the server. One small POSIX `sh` script prints the raw sources,
//! each under an `@name` marker line, and the parsers here turn them into numbers. Linux is
//! read from `/proc`; macOS and the BSDs from `sysctl`, `vm_stat`, `netstat` and `ps`.
//! Sources a server doesn't have are simply left out.

use serde::Serialize;

/// Run as `sh -c '…'` so it works whatever the login shell is (fish, csh…). It must not
/// contain single quotes.
pub const SCRIPT: &str = r#"LC_ALL=C; export LC_ALL
echo @os; uname -s
echo @host; uname -n
echo @kernel; uname -r
if [ -r /proc/stat ]; then
  echo @release; (. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")
  echo @stat; head -n 1 /proc/stat
  echo @meminfo; cat /proc/meminfo
  echo @uptime; cat /proc/uptime
  echo @loadavg; cat /proc/loadavg
  echo @ncpu; grep -c ^processor /proc/cpuinfo
  echo @netdev; cat /proc/net/dev
else
  echo @release; sw_vers -productName 2>/dev/null; sw_vers -productVersion 2>/dev/null
  echo @ncpu; sysctl -n hw.ncpu
  echo @memsize; sysctl -n hw.memsize 2>/dev/null || sysctl -n hw.physmem
  echo @vmstat; vm_stat 2>/dev/null
  echo @boottime; sysctl -n kern.boottime
  echo @now; date +%s
  echo @vmload; sysctl -n vm.loadavg
  echo @netstat; netstat -ibn 2>/dev/null
fi
echo @ps; ps -A -o pid= -o pcpu= -o pmem= -o comm= 2>/dev/null
echo @df; df -Pk 2>/dev/null
"#;

/// How many processes the panel lists.
const TOP: usize = 5;

/// The full command to pass to `exec`.
pub fn command() -> String {
    format!("sh -c '{SCRIPT}'")
}

/// Cumulative CPU time from `/proc/stat`. The UI turns two samples into percentages.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct CpuTicks {
    pub total: u64,
    pub idle: u64,
    /// user + nice
    pub user: u64,
    /// system + irq + softirq
    pub system: u64,
    pub iowait: u64,
    pub steal: u64,
}

/// Bytes received / sent on all interfaces except loopback since boot.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct NetBytes {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Disk {
    pub mount: String,
    pub size: u64,
    pub used: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub cpu: f64,
    pub mem: f64,
}

#[derive(Serialize, Debug, Default, Clone, PartialEq)]
pub struct HostStats {
    /// `uname -s`: Linux, Darwin, FreeBSD…
    pub os: String,
    pub hostname: String,
    pub kernel: String,
    /// "Ubuntu 24.04.1 LTS", "macOS 15.1"
    pub release: String,
    /// Linux: counters to diff between two samples.
    pub cpu_ticks: Option<CpuTicks>,
    /// Elsewhere: an instant estimate in percent (sum of `ps` %cpu over the core count).
    pub cpu_percent: Option<f64>,
    pub cpus: Option<u32>,
    /// Bytes.
    pub mem_total: Option<u64>,
    pub mem_used: Option<u64>,
    /// Page cache and buffers the kernel gives back when needed (not counted in `mem_used`).
    pub mem_cache: Option<u64>,
    pub swap_total: Option<u64>,
    pub swap_used: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub load: Option<[f64; 3]>,
    pub procs: Option<u32>,
    pub net: Option<NetBytes>,
    pub disks: Vec<Disk>,
    /// Busiest processes by CPU.
    pub top: Vec<Proc>,
}

impl HostStats {
    /// Anything worth showing came back.
    pub fn is_empty(&self) -> bool {
        self.cpu_ticks.is_none()
            && self.cpu_percent.is_none()
            && self.mem_total.is_none()
            && self.uptime_secs.is_none()
            && self.load.is_none()
            && self.disks.is_empty()
    }
}

/// Split the script output into `(marker, body)` sections.
fn sections(out: &str) -> Vec<(&str, Vec<&str>)> {
    let mut v: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) =
            line.strip_prefix('@').filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase()))
        {
            v.push((name, Vec::new()));
        } else if let Some((_, body)) = v.last_mut() {
            body.push(line);
        }
    }
    v
}

pub fn parse(out: &str) -> HostStats {
    let mut s = HostStats::default();
    let mut boot = None;
    let mut now = None;
    let mut page = 4096u64;
    let mut vm = None;
    let mut ps = Vec::new();
    for (name, body) in sections(out) {
        let first = body.iter().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or("");
        match name {
            "os" => s.os = first.to_string(),
            "host" => s.hostname = first.to_string(),
            "kernel" => s.kernel = first.to_string(),
            "release" => {
                s.release =
                    body.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
            }
            "stat" => s.cpu_ticks = proc_stat(first),
            "meminfo" => meminfo(&body, &mut s),
            "uptime" => {
                s.uptime_secs =
                    first.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()).map(|x| x as u64)
            }
            "loadavg" => {
                s.load = three_floats(first);
                // `0.42 0.31 0.25 1/123 4567`: running/total
                s.procs = first.split_whitespace().nth(3).and_then(|x| x.split_once('/')?.1.parse().ok());
            }
            "ncpu" => s.cpus = first.parse().ok().filter(|&n| n > 0),
            "memsize" => s.mem_total = first.parse().ok(),
            "vmstat" => {
                if let Some(p) = body.first().and_then(|l| vm_page_size(l)) {
                    page = p;
                }
                vm = Some(body);
            }
            "boottime" => boot = boottime(first),
            "now" => now = first.parse::<u64>().ok(),
            "vmload" => s.load = three_floats(first.trim_matches(|c| c == '{' || c == '}' || c == ' ')),
            "netdev" => s.net = proc_net_dev(&body),
            "netstat" => s.net = netstat(&body),
            "ps" => ps = procs(&body),
            "df" => s.disks = df(&body),
            _ => {}
        }
    }
    if let (Some(b), Some(n)) = (boot, now) {
        s.uptime_secs = n.checked_sub(b);
    }
    if let (Some(total), Some(body)) = (s.mem_total, vm) {
        let used = (vm_pages(&body, &["Pages active", "Pages wired down", "Pages occupied by compressor"])
            * page)
            .min(total);
        s.mem_used = Some(used);
        s.mem_cache = Some((vm_pages(&body, &["File-backed pages"]) * page).min(total - used));
    }
    if !ps.is_empty() {
        if s.cpu_ticks.is_none() {
            if let Some(n) = s.cpus {
                let sum: f64 = ps.iter().map(|p| p.cpu).sum();
                s.cpu_percent = Some((sum / n as f64).clamp(0.0, 100.0));
            }
        }
        if s.procs.is_none() {
            s.procs = Some(ps.len() as u32);
        }
        ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.total_cmp(&a.mem)));
        ps.truncate(TOP);
        s.top = ps;
    }
    s
}

/// `cpu  user nice system idle iowait irq softirq steal guest guest_nice`
fn proc_stat(line: &str) -> Option<CpuTicks> {
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let n: Vec<u64> = it.filter_map(|x| x.parse().ok()).collect();
    if n.len() < 4 {
        return None;
    }
    let at = |i: usize| n.get(i).copied().unwrap_or(0);
    // guest time is already counted in user / nice
    Some(CpuTicks {
        total: n.iter().take(8).sum(),
        idle: at(3) + at(4),
        user: at(0) + at(1),
        system: at(2) + at(5) + at(6),
        iowait: at(4),
        steal: at(7),
    })
}

fn meminfo(body: &[&str], s: &mut HostStats) {
    let kb = |key: &str| {
        body.iter().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.split_whitespace().next()?.parse::<u64>().ok()).flatten()
        })
    };
    let Some(total) = kb("MemTotal") else { return };
    let cache = kb("Buffers").unwrap_or(0) + kb("Cached").unwrap_or(0) + kb("SReclaimable").unwrap_or(0);
    // MemAvailable since Linux 3.14; older kernels: free + buffers + cache
    let avail = kb("MemAvailable").unwrap_or_else(|| kb("MemFree").unwrap_or(0) + cache);
    let used = total.saturating_sub(avail);
    s.mem_total = Some(total * 1024);
    s.mem_used = Some(used * 1024);
    s.mem_cache = Some(cache.min(total - used) * 1024);
    if let Some(st) = kb("SwapTotal").filter(|&t| t > 0) {
        s.swap_total = Some(st * 1024);
        s.swap_used = Some(st.saturating_sub(kb("SwapFree").unwrap_or(st)) * 1024);
    }
}

fn three_floats(s: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = s.split_whitespace().take(3).filter_map(|x| x.replace(',', ".").parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// `/proc/net/dev`: `  eth0: rx_bytes rx_packets … (8 rx fields) tx_bytes …`
fn proc_net_dev(body: &[&str]) -> Option<NetBytes> {
    let mut n = NetBytes { rx: 0, tx: 0 };
    let mut any = false;
    for l in body {
        let Some((name, rest)) = l.split_once(':') else { continue };
        let name = name.trim();
        if name == "lo" || name.contains('|') {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if f.len() >= 9 {
            n.rx += f[0];
            n.tx += f[8];
            any = true;
        }
    }
    any.then_some(n)
}

/// `netstat -ibn`: `Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll`.
/// One `<Link#n>` row per interface carries its totals; the Address column may be empty.
fn netstat(body: &[&str]) -> Option<NetBytes> {
    let mut lines = body.iter().skip_while(|l| !l.trim_start().starts_with("Name"));
    let head: Vec<&str> = lines.next()?.split_whitespace().collect();
    let col = |name: &str| head.iter().position(|h| *h == name);
    let (ib, ob) = (col("Ibytes")?, col("Obytes")?);
    let mut n = NetBytes { rx: 0, tx: 0 };
    let mut any = false;
    let mut seen = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || !f[2].starts_with("<Link") || f[0].starts_with("lo") || seen.contains(&f[0]) {
            continue;
        }
        // without an address every column after Network moves one to the left
        let shift = head.len().saturating_sub(f.len()).min(1);
        let num = |i: usize| f.get(i - shift).and_then(|x| x.parse::<u64>().ok());
        if let (Some(r), Some(t)) = (num(ib), num(ob)) {
            n.rx += r;
            n.tx += t;
            any = true;
            seen.push(f[0]);
        }
    }
    any.then_some(n)
}

/// `ps -A -o pid= -o pcpu= -o pmem= -o comm=`
fn procs(body: &[&str]) -> Vec<Proc> {
    body.iter()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let cpu = it.next()?.replace(',', ".").parse().ok()?;
            let mem = it.next()?.replace(',', ".").parse().ok()?;
            let comm = it.collect::<Vec<_>>().join(" ");
            // macOS prints the full path; Linux names such as `kworker/0:1` keep their slash
            let name = if comm.starts_with('/') { comm.rsplit('/').next().unwrap_or(&comm) } else { &comm };
            (!name.is_empty()).then(|| Proc { pid, name: name.to_string(), cpu, mem })
        })
        .collect()
}

/// `Mach Virtual Memory Statistics: (page size of 16384 bytes)`
fn vm_page_size(line: &str) -> Option<u64> {
    let i = line.find("page size of ")? + "page size of ".len();
    line[i..].split_whitespace().next()?.parse().ok()
}

/// Sum of `vm_stat` page counts. In use, the way Activity Monitor counts it: active + wired + compressed.
fn vm_pages(body: &[&str], keys: &[&str]) -> u64 {
    keys.iter()
        .map(|key| {
            body.iter()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    (k.trim() == *key).then(|| v.trim().trim_end_matches('.').parse::<u64>().ok()).flatten()
                })
                .unwrap_or(0)
        })
        .sum()
}

/// `{ sec = 1700000000, usec = 123 } Tue Nov 14 …`
fn boottime(s: &str) -> Option<u64> {
    let i = s.find("sec =")? + "sec =".len();
    s[i..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// Pseudo and system mounts that only add noise.
fn skip_mount(dev: &str, mount: &str) -> bool {
    const DEVS: &[&str] =
        &["tmpfs", "devtmpfs", "udev", "none", "shm", "devfs", "map", "proc", "sysfs", "cgroup"];
    const MOUNTS: &[&str] = &[
        "/proc",
        "/sys",
        "/dev",
        "/run",
        "/snap",
        "/boot/efi",
        "/private/var/vm",
        "/Library/Developer/CoreSimulator",
        "/var/lib/docker",
        "/var/lib/containers",
    ];
    // `overlay` is the root of a container; on a Docker host it is each container's root
    if DEVS.contains(&dev) || dev.starts_with("/dev/loop") || dev.starts_with("map ") {
        return true;
    }
    if MOUNTS.iter().any(|m| mount == *m || mount.starts_with(&format!("{m}/"))) {
        return true;
    }
    // macOS: the system volume's helpers; the Data volume holds the user's files
    mount.starts_with("/System/Volumes/") && mount != "/System/Volumes/Data"
}

/// `df -Pk`: `Filesystem 1024-blocks Used Available Capacity Mounted on`
fn df(body: &[&str]) -> Vec<Disk> {
    let mut v: Vec<Disk> = Vec::new();
    for l in body.iter().skip_while(|l| !l.starts_with("Filesystem")).skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        // the device name may contain spaces: count fields from the end, mount point last
        let Some(cap) = f.iter().position(|x| x.ends_with('%')) else { continue };
        if cap < 4 {
            continue;
        }
        let num = |i: usize| f[i].parse::<u64>().ok();
        let (Some(size), Some(used)) = (num(cap - 3), num(cap - 2)) else { continue };
        let dev = f[..cap - 3].join(" ");
        let mount = f[cap + 1..].join(" ");
        if size == 0 || mount.is_empty() || skip_mount(&dev, &mount) {
            continue;
        }
        // the same device mounted twice (bind mounts): keep the first
        if v.iter().any(|d| d.size == size * 1024 && d.used == used * 1024) {
            continue;
        }
        v.push(Disk { mount, size: size * 1024, used: used * 1024 });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "@os
Linux
@host
web-01
@kernel
6.8.0-45-generic
@release
Ubuntu 24.04.1 LTS
@stat
cpu  4705 356 584 3699176 23 0 27 0 0 0
@meminfo
MemTotal:        8000000 kB
MemFree:         1000000 kB
MemAvailable:    6000000 kB
Buffers:          100000 kB
Cached:          2000000 kB
SwapTotal:       2000000 kB
SwapFree:        1500000 kB
@uptime
350735.47 234388.90
@loadavg
0.42 0.31 0.25 1/123 4567
@ncpu
4
@netdev
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 9999999   100    0    0    0     0          0         0 9999999   100    0    0    0     0       0          0
  eth0: 1000000  2000    0    0    0     0          0         0  500000  1500    0    0    0     0       0          0
docker0:   20000    30    0    0    0     0          0         0   10000    20    0    0    0     0       0          0
@ps
    1  0.0  0.1 systemd
  812 12.5  4.2 postgres
  900  0.3  9.8 java
 1201 55.0  1.0 kworker/0:1-events
 1300  0.0  0.0 ps
@df
Filesystem     1024-blocks    Used Available Capacity Mounted on
udev               4000000       0   4000000       0% /dev
tmpfs               800000    1000    799000       1% /run
/dev/sda1         50000000 20000000  30000000      40% /
/dev/sda1         50000000 20000000  30000000      40% /var/lib/docker/bind
/dev/loop0           60000   60000         0     100% /snap/core/123
/dev/sdb1        100000000 50000000  50000000      50% /mnt/My Data
";

    #[test]
    fn linux() {
        let s = parse(LINUX);
        assert_eq!(s.os, "Linux");
        assert_eq!(
            s.cpu_ticks,
            Some(CpuTicks {
                total: 4705 + 356 + 584 + 3699176 + 23 + 27,
                idle: 3699176 + 23,
                user: 4705 + 356,
                system: 584 + 27,
                iowait: 23,
                steal: 0,
            })
        );
        assert_eq!((s.hostname.as_str(), s.kernel.as_str()), ("web-01", "6.8.0-45-generic"));
        assert_eq!(s.release, "Ubuntu 24.04.1 LTS");
        assert_eq!(s.mem_cache, Some(2_100_000 * 1024));
        assert_eq!(s.procs, Some(123));
        assert_eq!(s.net, Some(NetBytes { rx: 1_020_000, tx: 510_000 }));
        let top: Vec<_> = s.top.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(top, ["kworker/0:1-events", "postgres", "java", "systemd", "ps"]);
        assert_eq!(s.top[1].pid, 812);
        assert_eq!(s.cpu_percent, None, "Linux CPU comes from /proc/stat");
        assert_eq!(s.cpus, Some(4));
        assert_eq!(s.mem_total, Some(8_000_000 * 1024));
        assert_eq!(s.mem_used, Some(2_000_000 * 1024));
        assert_eq!(s.swap_total, Some(2_000_000 * 1024));
        assert_eq!(s.swap_used, Some(500_000 * 1024));
        assert_eq!(s.uptime_secs, Some(350735));
        assert_eq!(s.load, Some([0.42, 0.31, 0.25]));
        let mounts: Vec<_> = s.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/mnt/My Data"]);
        assert_eq!(s.disks[0].used, 20_000_000 * 1024);
        assert!(!s.is_empty());
    }

    #[test]
    fn container_root_is_overlay() {
        let s = parse(
            "@df
Filesystem     1024-blocks     Used Available Capacity Mounted on
overlay         1055761844 49954728 952103692       5% /
tmpfs                65536        0     65536       0% /dev
shm                  65536        0     65536       0% /dev/shm
/dev/vda1       1055761844 49954728 952103692       5% /config
overlay           20000000  1000000  19000000       5% /var/lib/docker/overlay2/abc/merged
",
        );
        let mounts: Vec<_> = s.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/"]);
    }

    #[test]
    fn old_kernel_without_memavailable() {
        let s = parse(
            "@meminfo\nMemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 250 kB\nSwapTotal: 0 kB\n",
        );
        assert_eq!(s.mem_used, Some(600 * 1024));
        assert_eq!(s.swap_total, None);
    }

    const MAC: &str = "@os
Darwin
@release
macOS
15.1
@ncpu
8
@memsize
17179869184
@vmstat
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               12345.
Pages active:                            300000.
Pages inactive:                          280000.
Pages wired down:                        100000.
Pages occupied by compressor:             50000.
File-backed pages:                       200000.
@boottime
{ sec = 1700000000, usec = 386147 } Tue Nov 14 22:13:20 2023
@now
1700086400
@vmload
{ 1,52 1,70 1,81 }
@netstat
Name       Mtu   Network       Address            Ipkts Ierrs     Ibytes    Opkts Oerrs     Obytes  Coll
lo0        16384 <Link#1>                       5283271     0 4320229665  5283271     0 4320229665     0
lo0        16384 127           127.0.0.1        5283271     - 4320229665  5283271     - 4320229665     -
gif0*      1280  <Link#2>                             0     0          0        0     0          0     0
en0        1500  <Link#11>   aa:bb:cc:dd:ee:ff  1000000     0 9000000000   800000     0 1000000000     0
en0        1500  192.168.1     192.168.1.20       1000000     - 9000000000   800000     - 1000000000     -
utun0      1380  <Link#20>                          500     0      70000      400     0      30000     0
@ps
    1  12.5  0.1 /sbin/launchd
  420   3,5  2.0 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome
  500   0.0  0.0 /usr/libexec/logd
@df
Filesystem     1024-blocks      Used Available Capacity  Mounted on
/dev/disk3s1s1   971350180  10000000 500000000     2%    /
devfs                  400       400         0   100%    /dev
/dev/disk3s6     971350180   2000000 500000000     1%    /System/Volumes/VM
/dev/disk3s5     971350180 400000000 500000000    45%    /System/Volumes/Data
map auto_home            0         0         0   100%    /System/Volumes/Data/home
";

    #[test]
    fn macos() {
        let s = parse(MAC);
        assert_eq!(s.os, "Darwin");
        assert_eq!(s.cpu_ticks, None);
        assert_eq!(s.cpu_percent, Some(2.0));
        assert_eq!(s.mem_total, Some(17179869184));
        assert_eq!(s.mem_used, Some(450_000 * 16384));
        assert_eq!(s.mem_cache, Some(200_000 * 16384));
        assert_eq!(s.release, "macOS 15.1");
        assert_eq!(s.net, Some(NetBytes { rx: 9_000_070_000, tx: 1_000_030_000 }));
        assert_eq!(s.procs, Some(3));
        assert_eq!(s.top[1].name, "Google Chrome");
        assert_eq!(s.uptime_secs, Some(86400));
        assert_eq!(s.load, Some([1.52, 1.70, 1.81]));
        let mounts: Vec<_> = s.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/System/Volumes/Data"]);
    }

    #[test]
    fn nothing_useful() {
        assert!(parse("").is_empty());
        assert!(parse("'sh' is not recognized as an internal or external command").is_empty());
        assert!(parse("@os\nLinux\n@stat\n@df\ndf: command not found\n").is_empty());
    }

    #[test]
    fn script_has_no_single_quotes() {
        assert!(!SCRIPT.contains('\''));
    }
}
