//! Puts `assets/lean.ico` into the exe as its icon (what Explorer, the taskbar and shortcuts show).
//!
//! The GNU toolchain has no resource compiler (`windres`), so this writes the resource object
//! itself: a COFF object with one `.rsrc` section holding the usual three-level resource tree
//! (type, id, language) with one RT_ICON per image in the .ico and an RT_GROUP_ICON listing them.

use std::path::{Path, PathBuf};

const ICON: &str = "assets/lean.ico";
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "windows" || env != "gnu" || arch != "x86_64" {
        return; // only the x86_64 GNU build is set up (and tested) for this
    }
    let ico = std::fs::read(ICON).expect("reading the icon");
    let resources = icon_resources(&ico);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("lean-icon.o");
    std::fs::write(&out, coff_object(&resources)).expect("writing the icon object");
    link(&out);
}

fn link(obj: &Path) {
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The .ico's images as RT_ICON resources 1..n, plus the RT_GROUP_ICON (id 1) that lists them.
fn icon_resources(ico: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    assert!(ico.len() >= 6 && u16_at(ico, 0) == 0 && u16_at(ico, 2) == 1, "{ICON} is not an .ico file");
    let count = u16_at(ico, 4) as usize;
    let mut group = Vec::new();
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&(count as u16).to_le_bytes());
    let mut res = Vec::new();
    for i in 0..count {
        let e = &ico[6 + 16 * i..6 + 16 * (i + 1)];
        let (size, offset) = (u32_at(e, 8) as usize, u32_at(e, 12) as usize);
        let id = (i + 1) as u16;
        res.push((RT_ICON, id, ico[offset..offset + size].to_vec()));
        // GRPICONDIRENTRY: the ICONDIRENTRY's first 12 bytes, then the RT_ICON id.
        group.extend_from_slice(&e[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    res.push((RT_GROUP_ICON, 1, group));
    res
}

/// A COFF object (x86_64) whose `.rsrc` section is the resource tree for `resources`
/// (type, id, data). Data entry addresses are RVAs, so each gets an ADDR32NB relocation.
fn coff_object(resources: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut types: Vec<u16> = resources.iter().map(|r| r.0).collect();
    types.sort();
    types.dedup();
    let ids_of = |t: u16| {
        let mut v: Vec<&(u16, u16, Vec<u8>)> = resources.iter().filter(|r| r.0 == t).collect();
        v.sort_by_key(|r| r.1);
        v
    };

    // Section layout: root dir, one dir per type, one language dir per resource, the data
    // entries, then the data.
    let dir_size = |entries: usize| 16 + 8 * entries;
    let mut type_dir_off = Vec::new();
    let mut off = dir_size(types.len());
    for &t in &types {
        type_dir_off.push(off);
        off += dir_size(ids_of(t).len());
    }
    let ordered: Vec<&(u16, u16, Vec<u8>)> = types.iter().flat_map(|&t| ids_of(t)).collect();
    let lang_dir_off: Vec<usize> = (0..ordered.len()).map(|i| off + i * dir_size(1)).collect();
    off += ordered.len() * dir_size(1);
    let data_entry_off: Vec<usize> = (0..ordered.len()).map(|i| off + i * 16).collect();
    off += ordered.len() * 16;
    let mut data_off = Vec::new();
    for r in &ordered {
        off = (off + 7) & !7;
        data_off.push(off);
        off += r.2.len();
    }
    let section_size = (off + 7) & !7;

    let mut sec = vec![0u8; section_size];
    let put16 = |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    // IMAGE_RESOURCE_DIRECTORY with `n` id entries; entries follow at +16.
    let dir = |b: &mut Vec<u8>, at: usize, n: usize| put16(b, at + 14, n as u16);
    const SUBDIR: u32 = 0x8000_0000;

    dir(&mut sec, 0, types.len());
    let mut k = 0;
    for (ti, &t) in types.iter().enumerate() {
        put32(&mut sec, 16 + 8 * ti, t as u32);
        put32(&mut sec, 16 + 8 * ti + 4, SUBDIR | type_dir_off[ti] as u32);
        let ids = ids_of(t);
        dir(&mut sec, type_dir_off[ti], ids.len());
        for (ii, r) in ids.iter().enumerate() {
            let e = type_dir_off[ti] + 16 + 8 * ii;
            put32(&mut sec, e, r.1 as u32);
            put32(&mut sec, e + 4, SUBDIR | lang_dir_off[k] as u32);
            dir(&mut sec, lang_dir_off[k], 1);
            put32(&mut sec, lang_dir_off[k] + 16, LANG_EN_US as u32);
            put32(&mut sec, lang_dir_off[k] + 20, data_entry_off[k] as u32);
            k += 1;
        }
    }
    for (i, r) in ordered.iter().enumerate() {
        // IMAGE_RESOURCE_DATA_ENTRY: RVA (section offset + relocation), size, code page, reserved.
        put32(&mut sec, data_entry_off[i], data_off[i] as u32);
        put32(&mut sec, data_entry_off[i] + 4, r.2.len() as u32);
        sec[data_off[i]..data_off[i] + r.2.len()].copy_from_slice(&r.2);
    }

    const HEADER: usize = 20;
    const SECTION_HEADER: usize = 40;
    let raw_at = HEADER + SECTION_HEADER;
    let relocs_at = raw_at + sec.len();
    let symbols_at = relocs_at + 10 * ordered.len();

    let mut o = Vec::new();
    // IMAGE_FILE_HEADER
    o.extend_from_slice(&0x8664u16.to_le_bytes()); // AMD64
    o.extend_from_slice(&1u16.to_le_bytes()); // sections
    o.extend_from_slice(&0u32.to_le_bytes()); // timestamp (0 keeps builds reproducible)
    o.extend_from_slice(&(symbols_at as u32).to_le_bytes());
    o.extend_from_slice(&2u32.to_le_bytes()); // the section symbol and its aux record
    o.extend_from_slice(&0u16.to_le_bytes()); // no optional header
    o.extend_from_slice(&0u16.to_le_bytes());
    // IMAGE_SECTION_HEADER
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(sec.len() as u32).to_le_bytes());
    o.extend_from_slice(&(raw_at as u32).to_le_bytes());
    o.extend_from_slice(&(relocs_at as u32).to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(ordered.len() as u16).to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.extend_from_slice(&0x4030_0040u32.to_le_bytes()); // initialized data, 4-byte aligned, read
    o.extend_from_slice(&sec);
    for &at in &data_entry_off {
        o.extend_from_slice(&(at as u32).to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes()); // symbol 0: the section
        o.extend_from_slice(&3u16.to_le_bytes()); // IMAGE_REL_AMD64_ADDR32NB
    }
    // Symbol `.rsrc` (static, section 1) with a section-definition aux record.
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.push(3);
    o.push(1);
    o.extend_from_slice(&(sec.len() as u32).to_le_bytes());
    o.extend_from_slice(&(ordered.len() as u16).to_le_bytes());
    o.extend_from_slice(&[0u8; 12]);
    // Empty string table.
    o.extend_from_slice(&4u32.to_le_bytes());
    o
}
