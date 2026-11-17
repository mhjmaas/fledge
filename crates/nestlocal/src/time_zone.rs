//! The unit's time zone. nlclient shows the time in the zone of the file
//! /etc/localtime links to (/media/user-config/localtime), which it wrote
//! from the zone Nest's service gave for the unit's location. nestlocal
//! gives no zone, so until one comes the unit keeps the zone it was
//! originally set to. Home Assistant (or any MQTT client) sends a zone to
//! <base>/set/time_zone (retained), and nestlocal installs the unit's
//! zoneinfo file for it there. nlclient reads the zone as it starts, so a
//! changed file means a reboot.
//!
//! The unit's zoneinfo is its firmware's (tzdata of about 2020): a zone
//! named or made since goes by the name it had then (OLDER_NAMES), and a
//! zone whose rules changed since keeps the old rules.

use std::io;
use std::path::{Path, PathBuf};

pub const ZONEINFO: &str = "/usr/share/zoneinfo";
pub const LOCALTIME: &str = "/etc/localtime";
/// The zone nestlocal last rebooted the unit into.
pub const MARKER: &str = "/media/data/nestlocal.time_zone";
/// Zones Home Assistant can name that the unit's tzdata lacks, and the zone
/// it has with the same rules today: two renames, and two zones split off
/// later (Ciudad Juárez from Ojinaga in 2022g, Coyhaique from Santiago in
/// 2025b).
const OLDER_NAMES: [(&str, &str); 4] = [
    ("Europe/Kyiv", "Europe/Kiev"),
    ("Pacific/Kanton", "Pacific/Enderbury"),
    ("America/Ciudad_Juarez", "America/Denver"),
    ("America/Coyhaique", "America/Punta_Arenas"),
];

/// What installing a zone did.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    /// The unit was already in it.
    Already,
    /// Changed: nlclient takes it as it starts.
    Changed,
    /// Put back after something else replaced it, nestlocal having
    /// rebooted the unit into it before: no second reboot for it.
    Restored,
}

/// An IANA zone name, as a relative path under the zoneinfo directory:
/// Europe/Amsterdam, UTC, America/Argentina/Buenos_Aires, Etc/GMT+1.
pub fn check_name(name: &str) -> Result<&str, String> {
    let ok = !name.is_empty() && name.len() <= 64 && name.split('/').all(|part| {
        !part.is_empty() && !part.starts_with('.')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'))
    });
    if ok { Ok(name) } else { Err(format!("expected a time zone name like Europe/Amsterdam, got {name:?}")) }
}

/// The file a link points to, or the path itself when it is no link.
fn target(link: &Path) -> PathBuf {
    match std::fs::read_link(link) {
        Ok(to) if to.is_absolute() => to,
        Ok(to) => link.parent().unwrap_or(Path::new("/")).join(to),
        Err(_) => link.to_path_buf(),
    }
}

/// The unit's zoneinfo file for `name`, under its older name when it lacks
/// this one.
fn zone_file(zoneinfo: &Path, name: &str) -> io::Result<Vec<u8>> {
    let older = OLDER_NAMES.iter().find(|(newer, _)| *newer == name).map(|(_, older)| *older);
    let zone = match (std::fs::read(zoneinfo.join(name)), older) {
        (Ok(zone), _) => zone,
        (Err(e), Some(older)) if e.kind() == io::ErrorKind::NotFound => std::fs::read(zoneinfo.join(older))
            .map_err(|e| io::Error::new(e.kind(), format!("no zone {name} or {older} on this unit: {e}")))?,
        (Err(e), _) => return Err(io::Error::new(e.kind(), format!("no zone {name} on this unit: {e}"))),
    };
    if zone.starts_with(b"TZif") { Ok(zone) } else { Err(io::Error::other(format!("{name}: not a zoneinfo file"))) }
}

/// Installs zone `name` from `zoneinfo` as the file `localtime` links to.
/// `marker` names the zone of the last change, so a file something else
/// keeps replacing is put back without a reboot each time.
pub fn install(zoneinfo: &Path, localtime: &Path, marker: &Path, name: &str) -> io::Result<Installed> {
    let name = check_name(name).map_err(io::Error::other)?;
    let zone = zone_file(zoneinfo, name)?;
    let file = target(localtime);
    if std::fs::read(&file).is_ok_and(|now| now == zone) {
        return Ok(Installed::Already);
    }
    let new = file.with_file_name(".localtime.nestlocal");
    std::fs::write(&new, &zone)?;
    std::fs::File::open(&new)?.sync_all()?;
    std::fs::rename(&new, &file)?;
    if std::fs::read_to_string(marker).is_ok_and(|last| last.trim() == name) {
        return Ok(Installed::Restored);
    }
    std::fs::write(marker, format!("{name}\n"))?;
    Ok(Installed::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nestlocal-time-zone-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("zoneinfo/Europe")).unwrap();
        std::fs::create_dir_all(dir.join("user-config")).unwrap();
        std::fs::write(dir.join("zoneinfo/Europe/Amsterdam"), b"TZif2 amsterdam\nCET-1CEST,M3.5.0,M10.5.0/3\n").unwrap();
        std::fs::write(dir.join("zoneinfo/Europe/London"), b"TZif2 london\nGMT0BST,M3.5.0/1,M10.5.0\n").unwrap();
        std::fs::write(dir.join("zoneinfo/Europe/Broken"), b"not a zone").unwrap();
        std::fs::write(dir.join("user-config/localtime"), b"TZif2 pacific\nPST8PDT,M3.2.0,M11.1.0\n").unwrap();
        std::os::unix::fs::symlink("user-config/localtime", dir.join("localtime")).unwrap();
        dir
    }

    #[test]
    fn names_are_zone_paths_and_nothing_else() {
        for ok in ["Europe/Amsterdam", "UTC", "America/Argentina/Buenos_Aires", "Etc/GMT+1", "Etc/GMT-14"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../../etc/shadow", "Europe/../../x", "Europe//Amsterdam", "Europe/Amsterdam/",
                    ".hidden", "Europe/Amster dam", "Europe\\Amsterdam", &"A".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn installs_the_zone_where_the_link_points_once() {
        let d = dir("install");
        let (zoneinfo, link, marker) = (d.join("zoneinfo"), d.join("localtime"), d.join("marker"));
        assert_eq!(install(&zoneinfo, &link, &marker, "Europe/Amsterdam").unwrap(), Installed::Changed);
        let file = d.join("user-config/localtime");
        assert_eq!(std::fs::read(&file).unwrap(), std::fs::read(zoneinfo.join("Europe/Amsterdam")).unwrap());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays a link");
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "Europe/Amsterdam\n");
        assert!(!d.join("user-config/.localtime.nestlocal").exists(), "the temporary file is left behind");
        // Home Assistant sends it again (retained, on every connect): nothing to do.
        assert_eq!(install(&zoneinfo, &link, &marker, "Europe/Amsterdam").unwrap(), Installed::Already);
        // Another zone is a change again.
        assert_eq!(install(&zoneinfo, &link, &marker, "Europe/London").unwrap(), Installed::Changed);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_zone_replaced_behind_its_back_goes_back_without_another_reboot() {
        let d = dir("restore");
        let (zoneinfo, link, marker) = (d.join("zoneinfo"), d.join("localtime"), d.join("marker"));
        assert_eq!(install(&zoneinfo, &link, &marker, "Europe/Amsterdam").unwrap(), Installed::Changed);
        std::fs::write(d.join("user-config/localtime"), b"TZif2 pacific\n").unwrap();
        assert_eq!(install(&zoneinfo, &link, &marker, "Europe/Amsterdam").unwrap(), Installed::Restored);
        assert_eq!(std::fs::read(d.join("user-config/localtime")).unwrap(),
                   std::fs::read(zoneinfo.join("Europe/Amsterdam")).unwrap());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_what_it_cannot_install_and_leaves_the_zone() {
        let d = dir("refuse");
        let (zoneinfo, link, marker) = (d.join("zoneinfo"), d.join("localtime"), d.join("marker"));
        let before = std::fs::read(d.join("user-config/localtime")).unwrap();
        assert!(install(&zoneinfo, &link, &marker, "Mars/Olympus_Mons").is_err());
        assert!(install(&zoneinfo, &link, &marker, "Europe/Broken").is_err());
        assert!(install(&zoneinfo, &link, &marker, "../user-config/localtime").is_err());
        assert_eq!(std::fs::read(d.join("user-config/localtime")).unwrap(), before);
        assert!(!marker.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_zone_newer_than_the_units_tzdata_goes_by_its_older_name() {
        let d = dir("older");
        let (zoneinfo, file, marker) = (d.join("zoneinfo"), d.join("user-config/localtime"), d.join("marker"));
        std::fs::write(zoneinfo.join("Europe/Kiev"), b"TZif2 kiev\nEET-2EEST,M3.5.0/3,M10.5.0/4\n").unwrap();
        assert_eq!(install(&zoneinfo, &file, &marker, "Europe/Kyiv").unwrap(), Installed::Changed);
        assert_eq!(std::fs::read(&file).unwrap(), std::fs::read(zoneinfo.join("Europe/Kiev")).unwrap());
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "Europe/Kyiv\n", "the name Home Assistant gave");
        assert_eq!(install(&zoneinfo, &file, &marker, "Europe/Kyiv").unwrap(), Installed::Already);
        // A unit that has the newer name uses it.
        std::fs::write(zoneinfo.join("Europe/Kyiv"), b"TZif2 kyiv\n").unwrap();
        assert_eq!(install(&zoneinfo, &file, &marker, "Europe/Kyiv").unwrap(), Installed::Restored);
        assert_eq!(std::fs::read(&file).unwrap(), b"TZif2 kyiv\n");
        // Neither: refused, naming both.
        let e = install(&zoneinfo, &file, &marker, "Pacific/Kanton").unwrap_err().to_string();
        assert!(e.contains("Pacific/Kanton") && e.contains("Pacific/Enderbury"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_plain_file_is_written_in_place() {
        let d = dir("plain");
        let (zoneinfo, file, marker) = (d.join("zoneinfo"), d.join("user-config/localtime"), d.join("marker"));
        assert_eq!(install(&zoneinfo, &file, &marker, "Europe/London").unwrap(), Installed::Changed);
        assert_eq!(std::fs::read(&file).unwrap(), std::fs::read(zoneinfo.join("Europe/London")).unwrap());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
