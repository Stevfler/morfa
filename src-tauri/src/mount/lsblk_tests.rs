use super::*;

fn devices(json: &str) -> Vec<(String, String, bool)> {
    candidates(&parse(json).unwrap()).into_iter().map(|d| (d.device, d.name, d.encrypted)).collect()
}

#[test]
fn real_machine_sample() {
    // Anonymised lsblk output from a dev box: LUKS root (unlocked), EFI, Windows
    // disk (EFI + MSR + C: + recovery), several NTFS data disks, zram swap.
    let got = devices(include_str!("fixtures/lsblk.json"));
    assert_eq!(
        got,
        [("/dev/nvme1n1p3".into(), String::new(), false), ("/dev/nvme0n1p2".into(), "Data M2".into(), false)]
    );
}

#[test]
fn luks_usb_and_old_lsblk() {
    let json = r#"{"blockdevices":[
      {"name":"sdd","type":"disk","rm":"1","hotplug":"1","size":"32000000000","children":[
        {"name":"sdd1","type":"part","fstype":"crypto_LUKS","size":"31000000000","mountpoint":null},
        {"name":"sdd2","type":"part","fstype":"exfat","label":"STICK","size":"900000000","mountpoint":"/run/media/u/STICK"}
      ]},
      {"name":"sde","type":"disk","rm":false,"hotplug":true,"size":64000000000,"fstype":"exfat","label":"CAM","mountpoints":[null]},
      {"name":"loop0","type":"loop","fstype":"squashfs","size":1000,"mountpoints":[]},
      {"name":"sdf","type":"disk","size":5,"children":[{"name":"sdf1","type":"part","fstype":"swap","size":5,"mountpoints":[]}]}
    ]}"#;
    let devs = parse(json).unwrap();
    let c = candidates(&devs);
    assert_eq!(c.len(), 2);
    assert!(c[0].encrypted && c[0].removable && c[0].device == "/dev/sdd1");
    assert!(c[1].removable && c[1].name == "CAM");
    let i = find_mounted(&devs, "/run/media/u/STICK").unwrap();
    assert_eq!(devs[root_of(&devs, i)].path, "/dev/sdd");
}

#[test]
fn udisks_output() {
    assert_eq!(parse_mounted("Mounted /dev/sdb1 at /run/media/u/My Disk\n").as_deref(), Some("/run/media/u/My Disk"));
    assert_eq!(parse_unlocked("Unlocked /dev/sda2 as /dev/dm-3.\n").as_deref(), Some("/dev/dm-3"));
    assert_eq!(
        parse_unlocked("Error unlocking /dev/sda2: GDBus.Error:org.freedesktop.UDisks2.Error.Failed: Device /dev/sda2 is already unlocked as /dev/dm-1").as_deref(),
        Some("/dev/dm-1")
    );
    assert_eq!(
        parse_already_mounted("Error mounting /dev/sdb1: GDBus.Error:org.freedesktop.UDisks2.Error.AlreadyMounted: Device /dev/sdb1 is already mounted at `/run/media/u/X'.\n").as_deref(),
        Some("/run/media/u/X")
    );
    assert_eq!(friendly_error("Error: GDBus.Error:org.freedesktop.UDisks2.Error.NotAuthorizedDismissed: Not authorized"), "Authorization was cancelled or denied");
    assert_eq!(friendly_error("Error unlocking /dev/sda2: Failed to activate device: Operation not permitted"), "Wrong passphrase");
    assert!(friendly_error("Error unmounting /dev/sdb1: target is busy").contains("busy"));
}
