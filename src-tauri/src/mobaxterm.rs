//! Import MobaXterm sessions from a `.mxtsessions` export (or the `[Bookmarks]` sections of
//! `MobaXterm.ini`).
//!
//! MobaXterm doesn't document the format; this follows the community description at
//! <https://gist.github.com/Ruzgfpegk/ab597838e4abbe8de30d7224afd062ea>. A folder is a
//! `[Bookmarks_N]` section with `SubRep=Folder\Sub`, and each session is one line:
//!
//! ```text
//! Name= #109#0%host%port%user%%x11%compression%command%gateways%ports%users%…#…#mode#comment#colour
//! ```
//!
//! Kept: host, port, user, command run at login, private key file (added to the keychain),
//! agent forwarding, the comment as notes, and SSH gateways as jump hosts. Folders become
//! groups. MobaXterm doesn't export passwords. Other session types are reported, not imported.

use crate::import::{KeyFiles, Report};
use crate::vault::{new_id, now, Host, VaultData};

/// Group for sessions that are not in a folder.
const GROUP: &str = "MobaXterm";

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Gateway {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: String,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    /// `SubRep` of the section the session is in ("" at the top level).
    pub folder: String,
    /// Session type: 0 SSH, 4 RDP, 5 VNC, 7 SFTP, 11 browser…
    pub kind: u32,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub command: String,
    pub x11: bool,
    /// SSH gateways, from the first hop to the last.
    pub gateways: Vec<Gateway>,
    pub key_file: String,
    pub agent_forward: bool,
    /// 0 none, 1 SOCKS 4, 2 SOCKS 5, 3 HTTP, 4 Telnet, 5 local command, 6/7 SSH.
    pub proxy: u32,
    pub comment: String,
}

/// MobaXterm's placeholders for characters that have a meaning in the session line.
fn unescape(s: &str) -> String {
    s.replace("__PTVIRG__", ";")
        .replace("__DBLQUO__", "\"")
        .replace("__PIPE__", "|")
        .replace("__PERCENT__", "%")
        .replace("__DIEZE__", "#")
}

/// Key paths store the drive letter as `_CurrentDrive_`.
fn key_path(s: &str) -> String {
    unescape(s.trim()).replace("_CurrentDrive_", "C")
}

/// A `__PIPE__`-separated list (gateway hosts, ports, users, keys); empty items are kept.
fn list(s: &str) -> Vec<&str> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split("__PIPE__").collect()
    }
}

/// `-1` is MobaXterm's "on".
fn on(s: &str) -> bool {
    s.trim() == "-1"
}

/// One session line (`value` is what follows `Name=`); None when it isn't a session.
pub fn session(name: &str, folder: &str, value: &str) -> Option<Session> {
    let parts: Vec<&str> = value.split('#').collect();
    let f: Vec<&str> = parts.get(2)?.split('%').collect();
    let get = |i: usize| f.get(i).copied().unwrap_or("");
    let kind = get(0).trim().parse().ok()?;
    let user = match get(3).trim() {
        "<default>" => "",
        u => u,
    };
    let (hosts, ports, users, keys) = (list(get(8)), list(get(9)), list(get(10)), list(get(15)));
    let gateways = hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| !h.trim().is_empty())
        .map(|(i, h)| Gateway {
            host: h.trim().to_string(),
            port: ports.get(i).and_then(|p| p.trim().parse().ok()).unwrap_or(22),
            user: users.get(i).map(|u| u.trim().to_string()).unwrap_or_default(),
            key_file: keys.get(i).map(|k| key_path(k)).unwrap_or_default(),
        })
        .collect();
    Some(Session {
        name: name.trim().to_string(),
        folder: folder.trim().to_string(),
        kind,
        host: get(1).trim().to_string(),
        port: get(2).trim().parse().unwrap_or(0),
        user: user.to_string(),
        command: unescape(get(7)),
        x11: on(get(5)),
        gateways,
        key_file: key_path(get(14)),
        agent_forward: on(get(34)),
        proxy: get(19).trim().parse().unwrap_or(0),
        comment: parts.get(5).map(|c| unescape(c).trim().to_string()).unwrap_or_default(),
    })
}

/// Sessions in a `.mxtsessions` or `MobaXterm.ini` file (other sections are ignored).
pub fn parse(text: &str) -> Vec<Session> {
    let mut out = Vec::new();
    let mut in_bookmarks = false;
    let mut folder = String::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            let section = &t[1..t.len() - 1];
            in_bookmarks = section == "Bookmarks" || section.starts_with("Bookmarks_");
            folder.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "SubRep" => folder = value.trim().to_string(),
            "ImgNum" | "" => {}
            name => out.extend(session(name, &folder, value)),
        }
    }
    out
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        4 => "RDP",
        5 => "VNC",
        7 => "SFTP",
        11 => "Browser",
        _ => "this type of",
    }
}

/// Group a session lands in: its folder path, or "MobaXterm" at the top level.
fn group_of(s: &Session) -> String {
    if s.folder.is_empty() {
        GROUP.to_string()
    } else {
        s.folder.split('\\').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" / ")
    }
}

/// A saved SSH host for a gateway, or a new one (behind `via`) when there is none.
fn gateway_host(
    data: &mut VaultData,
    g: &Gateway,
    via: &str,
    group: &str,
    keys: &mut KeyFiles,
    r: &mut Report,
    name: &str,
) -> String {
    let found = data.hosts.iter().find(|h| {
        h.protocol == "ssh"
            && h.host == g.host
            && h.addr_port() == g.port
            && (g.user.is_empty() || h.username == g.user)
    });
    if let Some(h) = found {
        return h.id.clone();
    }
    let label = if g.user.is_empty() { g.host.clone() } else { format!("{}@{}", g.user, g.host) };
    let h = Host {
        id: new_id(),
        label: label.clone(),
        group: group.to_string(),
        protocol: "ssh".into(),
        host: g.host.clone(),
        port: g.port,
        username: g.user.clone(),
        key_id: keys.id_for(data, &g.key_file, &label, r),
        jump_host: via.to_string(),
        created_at: now(),
        ..Default::default()
    };
    r.notes.push(format!("{name}: jump host {label} was added as a host"));
    let id = h.id.clone();
    data.hosts.push(h);
    id
}

/// Add `sessions` to the vault, folders as groups.
pub fn import(data: &mut VaultData, sessions: Vec<Session>) -> Report {
    let mut r = Report::default();
    let mut keys = KeyFiles::default();
    for s in sessions {
        if s.kind != 0 {
            r.skipped.push(format!("{}: {} sessions aren't supported", s.name, kind_name(s.kind)));
            continue;
        }
        if s.host.is_empty() {
            r.skipped.push(format!("{}: no host", s.name));
            continue;
        }
        let group = group_of(&s);
        if data.hosts.iter().any(|h| h.group == group && h.label == s.name) {
            r.existing += 1;
            continue;
        }
        let mut jump = String::new();
        for g in &s.gateways {
            jump = gateway_host(data, g, &jump, &group, &mut keys, &mut r, &s.name);
        }
        if s.x11 {
            r.notes.push(format!("{}: X11 forwarding isn't supported, so it was left off", s.name));
        }
        if s.proxy != 0 {
            let p = match s.proxy {
                1 => "SOCKS 4",
                2 => "SOCKS 5",
                3 => "HTTP",
                4 => "Telnet",
                5 => "local command",
                _ => "SSH",
            };
            r.notes.push(format!(
                "{}: {p} proxy not imported (K-Shell connects through SSH jump hosts only)",
                s.name
            ));
        }
        let h = Host {
            id: new_id(),
            label: s.name.clone(),
            group,
            protocol: "ssh".into(),
            host: s.host.clone(),
            port: if s.port == 0 { 22 } else { s.port },
            username: s.user.clone(),
            key_id: keys.id_for(data, &s.key_file, &s.name, &mut r),
            jump_host: jump,
            startup_command: s.command.clone(),
            agent_forward: s.agent_forward,
            notes: s.comment.clone(),
            created_at: now(),
            ..Default::default()
        };
        data.hosts.push(h);
        r.imported += 1;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    // Field positions from the format description; the first line has every SSH field up
    // to agent forwarding, the others are shorter like sessions saved by older versions.
    const FILE: &str = "[Bookmarks]\r\n\
SubRep=\r\n\
ImgNum=42\r\n\
Web= #109#0%10.0.0.5%2200%deploy%%0%-1%cd /srv__PTVIRG__ ls%%%%0%0%0%_CurrentDrive_:\\keys\\web.ppk%%-1%0%%0%%1080%%0%0%1%%0%%%%0%-1%-1%-1#MobaFont%10#0# #-1\r\n\
Inner= #109#0%10.1.0.9%22%root%%-1%0%%bastion.example.com__PIPE__10.0.0.1%22__PIPE__2222%ops__PIPE__%0%0%0%%%#MobaFont%10#0#db __DIEZE__1#-1\r\n\
\r\n\
[Bookmarks_1]\r\n\
SubRep=Clients\\Acme\r\n\
ImgNum=41\r\n\
Acme prod= #109#0%acme.example.com%22%<default>%%0%0%%%%%0%0%0%%%0%0%%2%proxy.local%1080%%#MobaFont%10#0# #-1\r\n\
Desktop= #91#4%10.0.0.7%3389%admin%0%0#MobaFont#0# #-1\r\n\
\r\n\
[Misc]\r\n\
Something=#109#0%nothere%22%x%\r\n";

    #[test]
    fn parses_sessions_and_folders() {
        let s = parse(FILE);
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Web", "Inner", "Acme prod", "Desktop"]);
        let web = &s[0];
        assert_eq!((web.host.as_str(), web.port, web.user.as_str()), ("10.0.0.5", 2200, "deploy"));
        assert_eq!(web.command, "cd /srv; ls");
        assert_eq!(web.key_file, r"C:\keys\web.ppk");
        assert!(web.agent_forward && !web.x11);
        let inner = &s[1];
        assert!(inner.x11);
        assert_eq!(inner.comment, "db #1");
        assert_eq!(
            inner.gateways,
            [
                Gateway {
                    host: "bastion.example.com".into(),
                    port: 22,
                    user: "ops".into(),
                    key_file: "".into()
                },
                Gateway { host: "10.0.0.1".into(), port: 2222, user: "".into(), key_file: "".into() },
            ]
        );
        assert_eq!((s[2].folder.as_str(), s[2].user.as_str(), s[2].proxy), (r"Clients\Acme", "", 2));
        assert_eq!(s[3].kind, 4);
    }

    #[test]
    fn imports_hosts_with_groups_and_jump_hosts() {
        let mut data = VaultData::default();
        let r = import(&mut data, parse(FILE));
        assert_eq!(r.imported, 3);
        assert_eq!(r.skipped, ["Desktop: RDP sessions aren't supported"]);
        let by = |l: &str| data.hosts.iter().find(|h| h.label == l).unwrap().clone();
        let web = by("Web");
        assert_eq!(
            (web.group.as_str(), web.port, web.startup_command.as_str()),
            ("MobaXterm", 2200, "cd /srv; ls")
        );
        assert!(web.password.is_empty() && web.agent_forward);
        assert!(r.notes.iter().any(|n| n.starts_with("Web: key file")), "{:?}", r.notes);
        assert_eq!(by("Acme prod").group, "Clients / Acme");
        assert!(r.notes.iter().any(|n| n.starts_with("Acme prod: SOCKS 5 proxy")), "{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.starts_with("Inner: X11")), "{:?}", r.notes);

        // gateways chain: Inner → 10.0.0.1:2222 → ops@bastion.example.com
        let inner = by("Inner");
        assert_eq!(inner.notes, "db #1");
        let last = data.hosts.iter().find(|h| h.id == inner.jump_host).unwrap();
        assert_eq!((last.host.as_str(), last.port), ("10.0.0.1", 2222));
        let first = data.hosts.iter().find(|h| h.id == last.jump_host).unwrap();
        assert_eq!((first.label.as_str(), first.username.as_str()), ("ops@bastion.example.com", "ops"));
        assert!(first.jump_host.is_empty());

        // importing again adds nothing, and the saved gateways are reused
        let n = data.hosts.len();
        let r = import(&mut data, parse(FILE));
        assert_eq!((r.imported, r.existing), (0, 3));
        assert_eq!(data.hosts.len(), n);
    }

    #[test]
    fn an_existing_host_is_used_as_the_gateway() {
        let mut data = VaultData::default();
        data.hosts.push(Host {
            id: "b".into(),
            label: "Bastion".into(),
            protocol: "ssh".into(),
            host: "bastion.example.com".into(),
            port: 22,
            username: "ops".into(),
            ..Default::default()
        });
        let line = "#109#0%10.1.0.9%22%root%%0%0%%bastion.example.com%22%ops%";
        import(&mut data, vec![session("Inner", "", line).unwrap()]);
        assert_eq!(data.hosts.len(), 2);
        assert_eq!(data.hosts[1].jump_host, "b");
    }
}
