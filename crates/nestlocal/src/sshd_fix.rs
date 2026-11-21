//! A unit Fledge v0.11.0 patched with SSH for keys only got
//! "PermitRootLogin prohibit-password" in sshd_config, which the Nest's
//! OpenSSH (6.0, before 7.0 knew it) refuses: sshd exits at boot and port 22
//! stays shut (fledge#16). The app writes "without-password" now, the older
//! name for the same; this puts it right on units patched before, at start
//! (before nlclient), and starts sshd where rcS starts it.

use std::path::Path;

pub const CONFIG: &str = "/etc/ssh/sshd_config";
pub const RCS: &str = "/etc/init.d/rcS";
const OLD: &str = "PermitRootLogin prohibit-password\n";
const NEW: &str = "PermitRootLogin without-password\n";
/// rcS's line with SSH on (the app comments it out with SSH off).
const STARTS_SSHD: &str = "\n${INITDIR}/sshd start\n";

/// Rewrites `config` if it holds the line OpenSSH 6 refuses; true when it
/// did and rcS (at `rcs`) starts sshd, so sshd wants starting now.
pub fn repair(config: &Path, rcs: &Path) -> std::io::Result<bool> {
    let Ok(text) = std::fs::read_to_string(config) else { return Ok(false) };
    if !text.contains(OLD) {
        return Ok(false);
    }
    let new = config.with_extension("fledge-new");
    std::fs::write(&new, text.replace(OLD, NEW))?;
    std::fs::rename(&new, config)?;
    Ok(std::fs::read_to_string(rcs).is_ok_and(|r| r.contains(STARTS_SSHD)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_only_line_openssh_6_refuses_is_put_right_and_sshd_started() {
        let dir = std::env::temp_dir().join(format!("fledge-sshd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (config, rcs) = (dir.join("sshd_config"), dir.join("rcS"));
        std::fs::write(&config, "Port 22\nPermitRootLogin prohibit-password\nUsePAM no\n").unwrap();
        std::fs::write(&rcs, "#!/bin/sh\n${INITDIR}/sshd start\n").unwrap();
        assert!(repair(&config, &rcs).unwrap());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "Port 22\nPermitRootLogin without-password\nUsePAM no\n");
        assert!(!repair(&config, &rcs).unwrap(), "once is enough");
        std::fs::write(&config, "PermitRootLogin prohibit-password\n").unwrap();
        std::fs::write(&rcs, "#!/bin/sh\n#${INITDIR}/sshd start\n").unwrap();
        assert!(!repair(&config, &rcs).unwrap(), "SSH off: put right, not started");
        assert!(!repair(&dir.join("absent"), &rcs).unwrap(), "no OpenSSH (a gen 2's dropbear)");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
