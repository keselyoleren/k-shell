//! Import hosts from a CSV file, for people who keep their servers in a spreadsheet.
//!
//! The first row names the columns, in any order and case: `name` (or label, title),
//! `host` (or hostname, address, ip), `port`, `user` (or username, login), `group`
//! (or folder), `tags`, `protocol` and `notes` (or description, comment). Without a header
//! row the columns are `name,host,port,user,group`. Commas or semicolons (what Excel writes
//! in many locales) separate the values. A password column is never imported.

use crate::import::Report;
use crate::vault::{new_id, now, Host, VaultData};

/// Protocols a CSV row can create: the ones that need nothing but an address and a user.
const PROTOCOLS: [&str; 4] = ["ssh", "telnet", "ftp", "ftps"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Col {
    Name,
    Host,
    Port,
    User,
    Group,
    Tags,
    Protocol,
    Notes,
    Password,
    Other,
}

fn column(header: &str) -> Col {
    let h: String = header.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    match h.as_str() {
        "name" | "label" | "title" | "alias" | "session" | "sessionname" => Col::Name,
        "host" | "hostname" | "address" | "ip" | "ipaddress" | "server" => Col::Host,
        "port" => Col::Port,
        "user" | "username" | "login" => Col::User,
        "group" | "folder" => Col::Group,
        "tags" | "tag" => Col::Tags,
        "protocol" => Col::Protocol,
        "notes" | "note" | "description" | "comment" => Col::Notes,
        "password" | "pass" | "pwd" => Col::Password,
        _ => Col::Other,
    }
}

/// Rows of a CSV text: quoted values may hold the separator, `""` and line breaks.
pub fn rows(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let sep = if first.matches(';').count() > first.matches(',').count() { ';' } else { ',' };
    let mut out = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                out.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        out.push(row);
    }
    out.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
    out
}

/// `user@host:port` typed into the host column.
fn split_address(s: &str) -> (String, String, Option<u16>) {
    let (user, rest) = match s.rsplit_once('@') {
        Some((u, h)) => (u.to_string(), h),
        None => (String::new(), s),
    };
    // A single colon is a port; IPv6 addresses have several.
    if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':').unwrap();
        if let Ok(p) = p.parse() {
            return (user, h.to_string(), Some(p));
        }
    }
    (user, rest.trim_matches(|c| c == '[' || c == ']').to_string(), None)
}

/// Add the hosts in a CSV text to the vault.
pub fn import(data: &mut VaultData, text: &str) -> Result<Report, String> {
    let all = rows(text);
    if all.is_empty() {
        return Err("The CSV file is empty".into());
    }
    let header: Vec<Col> = all[0].iter().map(|h| column(h)).collect();
    let (cols, body, first_line) = if header.contains(&Col::Host) {
        (header, &all[1..], 2)
    } else {
        (vec![Col::Name, Col::Host, Col::Port, Col::User, Col::Group], &all[..], 1)
    };
    let mut r = Report::default();
    if cols.contains(&Col::Password) {
        r.notes
            .push("Passwords: the password column was not imported. Add passwords in the host editor".into());
    }
    for (i, row) in body.iter().enumerate() {
        let get = |want: Col| {
            cols.iter().position(|c| *c == want).and_then(|i| row.get(i)).map(|s| s.trim()).unwrap_or("")
        };
        let line = first_line + i;
        let (mut user, host, inline_port) = split_address(get(Col::Host));
        let name = match get(Col::Name) {
            "" => host.clone(),
            n => n.to_string(),
        };
        let what = if name.is_empty() { format!("Row {line}") } else { format!("Row {line} ({name})") };
        if host.is_empty() {
            r.skipped.push(format!("{what}: no host"));
            continue;
        }
        let protocol = match get(Col::Protocol).to_lowercase().as_str() {
            "" => "ssh".to_string(),
            p => p.to_string(),
        };
        if !PROTOCOLS.contains(&protocol.as_str()) {
            r.skipped.push(format!("{what}: {protocol} hosts can't be imported from CSV"));
            continue;
        }
        let port = match get(Col::Port) {
            "" => inline_port.unwrap_or(0),
            p => p.parse().unwrap_or_else(|_| {
                r.notes.push(format!("{what}: port \"{p}\" isn't a number, the default is used"));
                0
            }),
        };
        if !get(Col::User).is_empty() {
            user = get(Col::User).to_string();
        }
        let h = Host {
            id: new_id(),
            label: name,
            group: get(Col::Group).to_string(),
            tags: get(Col::Tags)
                .split([',', ';', '|'])
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect(),
            protocol,
            host,
            port,
            username: user,
            notes: get(Col::Notes).to_string(),
            created_at: now(),
            ..Default::default()
        };
        let same = data.hosts.iter().any(|x| {
            x.label == h.label
                && x.host == h.host
                && x.protocol == h.protocol
                && x.username == h.username
                && x.addr_port() == h.addr_port()
        });
        if same {
            r.existing += 1;
            continue;
        }
        data.hosts.push(h);
        r.imported += 1;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_values_and_semicolons() {
        let r = rows("a,\"b, c\",\"say \"\"hi\"\"\"\r\n\r\n1,\"two\nlines\",3\n");
        assert_eq!(r, [vec!["a", "b, c", "say \"hi\""], vec!["1", "two\nlines", "3"]]);
        assert_eq!(rows("Host;Port\nx;22"), [vec!["Host", "Port"], vec!["x", "22"]]);
    }

    #[test]
    fn imports_with_a_header_in_any_order() {
        let csv = "Group,Host Name,Label,User,Port,Tags,Password,Protocol\n\
prod,10.0.0.5,web,deploy,2200,\"nginx, eu\",hunter2,\n\
,ops@bastion.example.com:2222,,,,,,\n\
lab,192.168.1.2,switch,,,,,telnet\n\
lab,10.0.0.9,desk,,,,,rdp\n\
,,,nobody,,,,\n";
        let mut data = VaultData::default();
        let r = import(&mut data, csv).unwrap();
        assert_eq!(r.imported, 3);
        assert_eq!(r.skipped, ["Row 5 (desk): rdp hosts can't be imported from CSV", "Row 6: no host"]);
        assert!(r.notes[0].starts_with("Passwords: the password column was not imported"));
        let web = &data.hosts[0];
        assert_eq!(
            (web.label.as_str(), web.group.as_str(), web.host.as_str(), web.port, web.username.as_str()),
            ("web", "prod", "10.0.0.5", 2200, "deploy")
        );
        assert_eq!(web.tags, ["nginx", "eu"]);
        assert!(web.password.is_empty());
        assert_eq!(web.protocol, "ssh");
        let b = &data.hosts[1];
        assert_eq!((b.label.as_str(), b.username.as_str(), b.port), ("bastion.example.com", "ops", 2222));
        assert_eq!(data.hosts[2].protocol, "telnet");

        let r = import(&mut data, csv).unwrap();
        assert_eq!((r.imported, r.existing), (0, 3));
    }

    #[test]
    fn without_a_header_columns_are_name_host_port_user_group() {
        let mut data = VaultData::default();
        let r = import(&mut data, "db;10.0.0.7;5022;admin;prod\n").unwrap();
        assert_eq!(r.imported, 1);
        let h = &data.hosts[0];
        assert_eq!(
            (h.label.as_str(), h.host.as_str(), h.port, h.username.as_str(), h.group.as_str()),
            ("db", "10.0.0.7", 5022, "admin", "prod")
        );
    }

    #[test]
    fn bad_port_and_empty_file() {
        let mut data = VaultData::default();
        let r = import(&mut data, "host,port\nsrv,abc\n").unwrap();
        assert_eq!(data.hosts[0].port, 0);
        assert_eq!(r.notes, ["Row 2 (srv): port \"abc\" isn't a number, the default is used"]);
        assert!(import(&mut data, "\n\n").is_err());
    }
}
