//! Startup guard: validate the compiled-in kernel struct offsets against the running kernel's BTF.
//!
//! The eBPF probe reads `file->f_mode`, `file->f_path`, and `linux_binprm->file` at hardcoded offsets
//! ([`bee_common::offsets`], target 6.8 x86_64). A wrong offset reads the wrong field and enforcement
//! fails **open**, so before attaching we parse `/sys/kernel/btf/vmlinux`, resolve those members'
//! real byte offsets, and refuse to run on any mismatch (constitution Principle I; closes the CO-RE
//! fail-open noted in research R5). aya exposes no public API for BTF member offsets, so this is a
//! small, dependency-free reader of the (stable) BTF on-disk format — see
//! <https://www.kernel.org/doc/html/latest/bpf/btf.html>.

/// BTF header magic (`0xEB9F`), stored native-endian; on x86_64 that is little-endian.
const BTF_MAGIC: u16 = 0xEB9F;
/// `BTF_KIND_STRUCT`.
const BTF_KIND_STRUCT: u32 = 4;
/// Path to the running kernel's BTF.
const VMLINUX_BTF: &str = "/sys/kernel/btf/vmlinux";

fn le_u16(b: &[u8], off: usize) -> Result<u16, String> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "BTF truncated".to_string())
}

fn le_u32(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "BTF truncated".to_string())
}

/// Trailing bytes after a `btf_type` header for `kind` with `vlen` members/params, needed to walk to
/// the next type. `None` for an unknown kind — the walk cannot continue safely, so we fail closed.
fn trailing_len(kind: u32, vlen: usize) -> Option<usize> {
    Some(match kind {
        1 => 4,                          // INT       (u32)
        2 => 0,                          // PTR
        3 => 12,                         // ARRAY     (btf_array)
        4 | 5 => vlen * 12,              // STRUCT/UNION (btf_member[])
        6 => vlen * 8,                   // ENUM      (btf_enum[])
        7..=12 => 0,                     // FWD/TYPEDEF/VOLATILE/CONST/RESTRICT/FUNC
        13 => vlen * 8,                  // FUNC_PROTO (btf_param[])
        14 => 4,                         // VAR       (btf_var)
        15 => vlen * 12,                 // DATASEC   (btf_var_secinfo[])
        16 => 0,                         // FLOAT
        17 => 4,                         // DECL_TAG  (btf_decl_tag)
        18 => 0,                         // TYPE_TAG
        19 => vlen * 12,                 // ENUM64    (btf_enum64[])
        _ => return None,
    })
}

/// Read a NUL-terminated string at `strs[name_off..]`.
fn string_at(strs: &[u8], name_off: u32) -> Result<&str, String> {
    let start = name_off as usize;
    let slice = strs.get(start..).ok_or_else(|| "BTF string offset out of range".to_string())?;
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    core::str::from_utf8(&slice[..end]).map_err(|_| "BTF string is not UTF-8".to_string())
}

/// Resolve the byte offset of each `(struct_name, member_name)` in `wanted` from a raw BTF blob.
/// Returns offsets positionally aligned with `wanted`. Errors on a parse problem, an unknown BTF
/// kind (can't walk further), or any requested field not being found.
///
/// Assumes `vmlinux`-style deduplicated BTF (one full definition per struct name). The first
/// `STRUCT` bearing a wanted name supplies its members.
pub fn probe_field_offsets(btf: &[u8], wanted: &[(&str, &str)]) -> Result<Vec<usize>, String> {
    if le_u16(btf, 0)? != BTF_MAGIC {
        return Err("not a little-endian BTF blob (bad magic)".into());
    }
    let hdr_len = le_u32(btf, 4)? as usize;
    let type_off = le_u32(btf, 8)? as usize;
    let type_len = le_u32(btf, 12)? as usize;
    let str_off = le_u32(btf, 16)? as usize;
    let str_len = le_u32(btf, 20)? as usize;

    let type_start = hdr_len + type_off;
    let str_start = hdr_len + str_off;
    let types = btf
        .get(type_start..type_start + type_len)
        .ok_or_else(|| "BTF type section out of range".to_string())?;
    let strs = btf
        .get(str_start..str_start + str_len)
        .ok_or_else(|| "BTF string section out of range".to_string())?;

    let mut found: Vec<Option<usize>> = vec![None; wanted.len()];
    let mut pos = 0usize;
    while pos + 12 <= types.len() {
        let name_off = le_u32(types, pos)?;
        let info = le_u32(types, pos + 4)?;
        let kind = (info >> 24) & 0x1f;
        let vlen = (info & 0xffff) as usize;
        let kflag = (info >> 31) & 1;
        let trailing = trailing_len(kind, vlen).ok_or_else(|| format!("unknown BTF kind {kind}"))?;
        let body = pos + 12;

        if kind == BTF_KIND_STRUCT {
            let sname = string_at(strs, name_off)?;
            // Only scan members if this struct name is still wanted (avoids the per-member work for
            // the tens of thousands of unrelated structs in vmlinux BTF).
            let interested =
                wanted.iter().enumerate().any(|(i, (s, _))| *s == sname && found[i].is_none());
            if interested {
                for m in 0..vlen {
                    let mp = body + m * 12;
                    let m_name_off = le_u32(types, mp)?;
                    let m_raw_off = le_u32(types, mp + 8)?;
                    let mname = string_at(strs, m_name_off)?;
                    // With the struct's kflag set, a member's low 24 bits are the bit offset and the
                    // high 8 are the bitfield size; otherwise the whole word is the bit offset.
                    let bit_off = if kflag == 1 { m_raw_off & 0x00ff_ffff } else { m_raw_off };
                    for (i, (s, mem)) in wanted.iter().enumerate() {
                        if found[i].is_none() && *s == sname && *mem == mname {
                            found[i] = Some((bit_off / 8) as usize);
                        }
                    }
                }
            }
        }

        pos = body + trailing;
        if found.iter().all(Option::is_some) {
            break;
        }
    }

    let mut out = Vec::with_capacity(wanted.len());
    for (i, f) in found.into_iter().enumerate() {
        match f {
            Some(o) => out.push(o),
            None => return Err(format!("BTF: {}.{} not found", wanted[i].0, wanted[i].1)),
        }
    }
    Ok(out)
}

/// Validate [`bee_common::offsets::VALIDATED`] against the running kernel's BTF. `Ok(())` if every
/// compiled offset matches; `Err` (fail-closed) if BTF is unreadable, unparsable, or any offset
/// disagrees. The error names each mismatch so the operator knows bee must be retargeted/rebuilt.
pub fn check_running() -> Result<(), String> {
    let btf = std::fs::read(VMLINUX_BTF).map_err(|e| format!("cannot read {VMLINUX_BTF}: {e}"))?;
    let wanted: Vec<(&str, &str)> =
        bee_common::offsets::VALIDATED.iter().map(|(s, m, _)| (*s, *m)).collect();
    let found = probe_field_offsets(&btf, &wanted)?;
    diff_offsets(&found, bee_common::offsets::VALIDATED)
}

/// Compare probed offsets (positionally aligned with `table`) against the compiled values. `Ok(())`
/// iff every offset matches; otherwise a descriptive error naming each mismatch.
fn diff_offsets(found: &[usize], table: &[(&str, &str, usize)]) -> Result<(), String> {
    let mut mismatches = Vec::new();
    for ((s, m, expected), got) in table.iter().zip(found.iter()) {
        if got != expected {
            mismatches.push(format!("{s}.{m}: compiled {expected}, kernel {got}"));
        }
    }
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "compiled kernel struct offsets do not match the running kernel ({}); \
             bee must be rebuilt/retargeted for this kernel (see bee_common::offsets)",
            mismatches.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal BTF blob containing one `STRUCT` with the given `(member_name, bit_offset)`
    /// members. `kflag` selects the member-offset encoding.
    fn synth_btf(struct_name: &str, members: &[(&str, u32)], kflag: bool) -> Vec<u8> {
        // String section: leading NUL, then each name NUL-terminated.
        let mut strs = vec![0u8];
        let name_off = |s: &str, strs: &mut Vec<u8>| -> u32 {
            let off = strs.len() as u32;
            strs.extend_from_slice(s.as_bytes());
            strs.push(0);
            off
        };
        let sname_off = name_off(struct_name, &mut strs);
        let member_offs: Vec<(u32, u32)> =
            members.iter().map(|(n, bit)| (name_off(n, &mut strs), *bit)).collect();

        // Type section: one btf_type (STRUCT) + its btf_member[].
        let mut types = Vec::new();
        let vlen = members.len() as u32;
        let info = (BTF_KIND_STRUCT << 24) | (vlen & 0xffff) | if kflag { 1 << 31 } else { 0 };
        types.extend_from_slice(&sname_off.to_le_bytes());
        types.extend_from_slice(&info.to_le_bytes());
        types.extend_from_slice(&200u32.to_le_bytes()); // struct size (unused by the parser)
        for (m_name_off, bit) in &member_offs {
            let raw = if kflag { bit & 0x00ff_ffff } else { *bit };
            types.extend_from_slice(&m_name_off.to_le_bytes());
            types.extend_from_slice(&1u32.to_le_bytes()); // member type id (unused)
            types.extend_from_slice(&raw.to_le_bytes());
        }

        // Header: magic, version, flags, hdr_len, type_off, type_len, str_off, str_len.
        let hdr_len = 24u32;
        let mut out = Vec::new();
        out.extend_from_slice(&BTF_MAGIC.to_le_bytes());
        out.push(1); // version
        out.push(0); // flags
        out.extend_from_slice(&hdr_len.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // type_off
        out.extend_from_slice(&(types.len() as u32).to_le_bytes()); // type_len
        out.extend_from_slice(&(types.len() as u32).to_le_bytes()); // str_off (after types)
        out.extend_from_slice(&(strs.len() as u32).to_le_bytes()); // str_len
        out.extend_from_slice(&types);
        out.extend_from_slice(&strs);
        out
    }

    #[test]
    fn resolves_member_byte_offsets() {
        // f_mode at bit 160 (byte 20), f_path at bit 1216 (byte 152) — the real 6.8 offsets.
        let btf = synth_btf("file", &[("f_mode", 160), ("f_path", 1216)], false);
        let got = probe_field_offsets(&btf, &[("file", "f_mode"), ("file", "f_path")]).unwrap();
        assert_eq!(got, vec![20, 152]);
    }

    #[test]
    fn honors_bitfield_kflag_encoding() {
        // With kflag set, the high 8 bits are a bitfield size and must be masked off.
        let raw = 160 | (3 << 24); // bit offset 160, bitfield size 3
        let btf = synth_btf("file", &[("f_mode", raw)], true);
        let got = probe_field_offsets(&btf, &[("file", "f_mode")]).unwrap();
        assert_eq!(got, vec![20]);
    }

    #[test]
    fn errors_when_member_missing() {
        let btf = synth_btf("file", &[("f_mode", 160)], false);
        let err = probe_field_offsets(&btf, &[("file", "f_path")]).unwrap_err();
        assert!(err.contains("file.f_path not found"), "{err}");
    }

    #[test]
    fn errors_on_bad_magic() {
        let mut btf = synth_btf("file", &[("f_mode", 160)], false);
        btf[0] = 0x00; // corrupt magic
        assert!(probe_field_offsets(&btf, &[("file", "f_mode")]).is_err());
    }

    #[test]
    fn diff_offsets_detects_mismatch() {
        let table = &[("file", "f_mode", 20usize), ("file", "f_path", 152usize)];
        assert!(diff_offsets(&[20, 152], table).is_ok());
        // A drifted kernel where f_path moved to byte 160 must be reported (fail-closed).
        let err = diff_offsets(&[20, 160], table).unwrap_err();
        assert!(err.contains("file.f_path: compiled 152, kernel 160"), "{err}");
        assert!(!err.contains("f_mode"), "unchanged field must not be flagged: {err}");
    }

    /// End-to-end of the guard's decision: a probed layout that differs from the compiled table
    /// makes the offsets gate fail, which forces `Support::is_supported()` to false.
    #[test]
    fn mismatch_makes_support_unsupported() {
        use crate::detect::Support;
        let drifted = synth_btf("file", &[("f_mode", 160 /* byte 20 */), ("f_path", 1280 /* byte 160, not 152 */)], false);
        let found = probe_field_offsets(&drifted, &[("file", "f_mode"), ("file", "f_path")]).unwrap();
        let offsets_ok =
            diff_offsets(&found, &[("file", "f_mode", 20), ("file", "f_path", 152)]).is_ok();
        assert!(!offsets_ok);
        let s = Support {
            kernel_ok: true,
            bpf_lsm_active: true,
            cgroup_v2: true,
            offsets_ok,
            kernel_version: Some((6, 8)),
            reason: None,
        };
        assert!(!s.is_supported(), "an offset mismatch must make the kernel unsupported");
    }
}
