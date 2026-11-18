//! The Chrome extension package format (CRX3): a header with signatures, then a zip. Pure code: parse and verify the header,
//! unpack the zip, give an unpacked extension its Web Store id. Downloading and installing is in `extensions.rs`.
//!
//! Layout: `Cr24`, version `3` (u32 LE), header length (u32 LE), the header (a protobuf `CrxFileHeader`), the zip.
//! The header has the signatures (`sha256_with_rsa` = field 2, `sha256_with_ecdsa` = field 3: each a public key, DER
//! `SubjectPublicKeyInfo`, and a signature) and `signed_header_data` (field 10000, which holds the 16-byte id). The signature
//! covers `"CRX3 SignedData\0" + u32 LE length of signed_header_data + signed_header_data + the zip`. The extension's id is the
//! first 16 bytes of the SHA-256 of the key that signed, so a package is genuine for an id when a key that hashes to the id signed it.

use crate::extensions::{base64_encode, id_from_bytes};
use std::io::Read;
use std::path::Path;

/// A package whose signature has been checked.
#[derive(Debug)]
pub struct Crx {
    /// The extension's id (the signing key's).
    #[cfg_attr(not(test), allow(dead_code))]
    pub id: String,
    /// The signing public key (DER `SubjectPublicKeyInfo`): the manifest's `key`, base64 encoded, keeps the id when unpacked.
    pub key: Vec<u8>,
    /// The zip with the extension's files.
    pub zip: Vec<u8>,
}

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// The length-delimited fields of a protobuf message as `(field number, bytes)`; other wire types are skipped.
fn proto_fields(buf: &[u8]) -> Result<Vec<(u64, &[u8])>, String> {
    let bad = || "the package header is damaged".to_owned();
    let (mut pos, mut out) = (0, Vec::new());
    while pos < buf.len() {
        let tag = varint(buf, &mut pos).ok_or_else(bad)?;
        match tag & 7 {
            0 => {
                varint(buf, &mut pos).ok_or_else(bad)?;
            }
            1 => pos += 8,
            5 => pos += 4,
            2 => {
                let len = varint(buf, &mut pos).ok_or_else(bad)? as usize;
                let end = pos.checked_add(len).filter(|e| *e <= buf.len()).ok_or_else(bad)?;
                out.push((tag >> 3, &buf[pos..end]));
                pos = end;
            }
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// One DER element: its tag, its content and what follows it.
fn der(buf: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = buf.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        (rest[..n].iter().fold(0usize, |a, b| a << 8 | *b as usize), &rest[n..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// The public key bits of a DER `SubjectPublicKeyInfo`, and whether it is an RSA key (else it must be an EC key).
fn spki_key(spki: &[u8]) -> Option<(bool, &[u8])> {
    const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    const EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    let (0x30, body, _) = der(spki)? else { return None };
    let (0x30, algorithm, rest) = der(body)? else { return None };
    let (0x06, oid, _) = der(algorithm)? else { return None };
    let (0x03, bits, _) = der(rest)? else { return None };
    let key = bits.strip_prefix(&[0])?;
    match oid {
        RSA => Some((true, key)),
        EC => Some((false, key)),
        _ => None,
    }
}

/// Is `signature` over `message` made by the key `spki` (RSA PKCS#1 v1.5 or ECDSA P-256, both with SHA-256)?
fn verifies(spki: &[u8], message: &[u8], signature: &[u8]) -> bool {
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1, RSA_PKCS1_2048_8192_SHA256};
    match spki_key(spki) {
        Some((true, key)) => UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key).verify(message, signature).is_ok(),
        Some((false, key)) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key).verify(message, signature).is_ok(),
        None => false,
    }
}

/// Read a CRX3 package and check that the key whose hash is its id signed it. `expected`: the id that was asked for.
pub fn parse(bytes: &[u8], expected: &str) -> Result<Crx, String> {
    if bytes.get(..4) != Some(b"Cr24") {
        return Err("not an extension package (no Cr24 header; the download may be an error page)".into());
    }
    let word = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if word(4) != Some(3) {
        return Err(format!("package version {:?} is not supported (only CRX3)", word(4)));
    }
    let header_len = word(8).ok_or("the package is cut short")? as usize;
    let header = bytes.get(12..12 + header_len).ok_or("the package is cut short")?;
    let zip = &bytes[12 + header_len..];
    let fields = proto_fields(header)?;
    let signed = fields.iter().find(|(n, _)| *n == 10000).map(|(_, b)| *b).ok_or("the package has no signed header data")?;
    let crx_id = proto_fields(signed)?.into_iter().find(|(n, _)| *n == 1).map(|(_, b)| b).ok_or("the package has no id")?;
    let id: String = crx_id.iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect();
    if id != expected {
        return Err(format!("the package is for the extension {id}, not {expected}"));
    }
    let mut message = b"CRX3 SignedData\0".to_vec();
    message.extend((signed.len() as u32).to_le_bytes());
    message.extend(signed);
    message.extend(zip);
    for (_, proof) in fields.iter().filter(|(n, _)| *n == 2 || *n == 3) {
        let parts = proto_fields(proof)?;
        let get = |n| parts.iter().find(|(f, _)| *f == n).map(|(_, b)| *b);
        let (Some(key), Some(signature)) = (get(1), get(2)) else { continue };
        if id_from_bytes(key) == id && verifies(key, &message, signature) {
            return Ok(Crx { id, key: key.to_vec(), zip: zip.to_vec() });
        }
    }
    Err(format!("no valid signature by the key of {id}: the package may have been tampered with"))
}

/// Most a package may unpack to, and most files in it (a guard against a zip bomb).
const MAX_UNPACKED: u64 = 400 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

/// Unpack the zip into `dest` (made if needed). File names that would leave `dest` are refused. Chrome's own `_metadata` folder is skipped.
pub fn unpack(zip: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| format!("the package's files cannot be read: {e}"))?;
    if archive.len() > MAX_FILES {
        return Err("the package has too many files".into());
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let rel = entry.enclosed_name().ok_or_else(|| format!("the package has a file name that leaves its folder: {}", entry.name()))?;
        if rel.starts_with("_metadata") {
            continue;
        }
        let target = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        total += std::io::copy(&mut entry.by_ref().take(MAX_UNPACKED.saturating_sub(total) + 1), &mut out).map_err(|e| e.to_string())?;
        if total > MAX_UNPACKED {
            return Err("the package unpacks to too much data".into());
        }
    }
    Ok(())
}

/// Give an unpacked extension the Web Store id: add the signing key to `manifest.json` as `"key"`. Web Store packages carry the
/// key in the header, not the manifest, and an unpacked extension with no `key` gets an id made from its folder, which
/// breaks anything that names the real id (1Password's native host only accepts its own ids). The text is edited, not rewritten,
/// so the manifest keeps its formatting. A manifest that already has a `key` is left alone.
pub fn add_key(manifest: &str, key: &[u8]) -> Result<String, String> {
    if serde_json::from_str::<serde_json::Value>(manifest).ok().is_some_and(|v| v.get("key").is_some()) {
        return Ok(manifest.to_owned());
    }
    let text = manifest.trim_start_matches('\u{feff}');
    let open = text.find('{').ok_or("the manifest is not an object")?;
    Ok(format!("{{\n  \"key\": \"{}\",{}", base64_encode(key), &text[open + 1..]))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, RsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING, RSA_PKCS1_SHA256};

    /// A throwaway RSA-2048 key (PKCS#8), made for these tests only.
    const RSA_TEST_KEY: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDEbCP9Shf/i1gNYZFkL4wTeEg5vtcW70WpY1SYpnIoVMCFd1MUwGthBNP+02qNeJNCyJhSisF04O3hjKQnhp1nJJ+08zl5cwxhRSxDMB2rIz9CcTjCXviVuYgEYXiRqnMFsG3cLQxAPf6RlpF/KpX7A7du1ZFAgq773mLFyjYyR2J3hQBGKcfM6JLFZYc7/fJ9g1W5+u7CNyMB/y7eh6U9VOwzczkAz5MjSiYuSxsMvBQ+SAuPM6tAR7tkuleMNbLxuvB0NBvuH6MD4OqdifrPiF61l9HqZg1L1MlpFrfxzcsCs13V+Qed2Dro/L9ngIz64ZthSNMuJAMhWqTlHq79AgMBAAECggEAA9eRVlDBn+oX7pO9k44E3tOf5rXs1fdtBNIbKJeRjOg9fM7Xn+9fRP5NGZiv5p+uEmTDP/1pU9HbgqmCLii+rEgQh1T+RznLdfKeWfEXaiPOHAyuvXSR8bciPzhbMqQ4zEAbd1lP5PN20YkLrwcETdeQhLk7i7K9XVKysBI+FaUmzILpWAoueXplWCSDrOqxI3NfjaHAsw7frXkq8SbxLls/gl91cDxFPaseRWrwDS6JLSTdMQSJqeWx0HP1vKlEbvQZn5qpfIci8Lhgm7m6/bYSKTmRhYVZ8WO+7aWS0niAXuIYMji9Ql8OKlNygyudrBk8Ll+WGS2aD9AezzWEsQKBgQD/as0Dtdcpsv5wDAVq/1NKZ5bvxPnXYjvmXqoWuCqRI91JQn3LhYOjdSb6aYnMDDkZPMDvhc+vN0cIoO1KGVbUUyAX6Ij/ur581wsXr1Qf5PaNoawwz7tHtCcnGOzHKMEotjijVHbnB5hDTOg3tmVkk1GsC/ulRxFthKu+/GVm7QKBgQDE3uDrv6nViUpYANi/1s/001Zjhr3TZdujFPW4gy5detoy99JNJCbpwFabJQTp3OXCzd0dF5LPuy5SLhIC4+rIaJzCzc/v2IY3dhQl88jGcSOXNI8aFqYE/9Wk1LLGdOCZKzgyFI0QknWUYcuBS2Y+WG/PdZD7whaDjl5C0hvWUQKBgEdSD0VF9XrTXHD1Kl4iRaO1IVR6cDlQWOGO9Xe6nUhB93yJcI7wDT4LbILoDW5Iy3jcmhNIDiIjEVa7eQcD5FcNjxyd9gNSpfY+Jf7LGnZ9fMs3pdlLaEQxbe1g7F0AaE9x8obJAxqn6ECCrSVBbTaUXG3qOq6AFcG7FioSWE5RAoGAWC2j/LPqM/S8UdJ/Qt3KY1RNe2Kcn5Ud3VvD+5L2L3SZSv7pORgaPdSidfcMzlrcpnZitWcboOOsCa8M7jJ7tKv2muJoYntPEoRW3fDlWszOLimidwLx9YdJf/ys3mr3NmO/Vd8Sl/uhooZzCkRlVUNzQS7dc4YpY19fJPrK8SECgYEAolxYXUrwhZkBzEvduP774Z6SAcp3clTxCR/dwebtOWlosPDHySgkjd5VjE5YfAXRDx1MAey7Zms5226g6oh/aFB7Jf1BuDgsg5ck3aeVD7TGEdUlHsfXHdlMI29fGmR9Ddd9sRcuJ6nhvyTXoJp571F2QNmOvJ9r0ZvhJPgF+4w=";

    fn der_wrap(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        match content.len() {
            n if n < 128 => out.push(n as u8),
            n if n < 256 => out.extend([0x81, n as u8]),
            n => out.extend([0x82, (n >> 8) as u8, n as u8]),
        }
        out.extend(content);
        out
    }

    fn spki(oid_and_params: &[u8], key: &[u8]) -> Vec<u8> {
        let mut bits = vec![0];
        bits.extend(key);
        let mut body = der_wrap(0x30, oid_and_params);
        body.extend(der_wrap(0x03, &bits));
        der_wrap(0x30, &body)
    }

    fn field(n: u64, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut tag = n << 3 | 2;
        loop {
            let b = (tag & 0x7f) as u8;
            tag >>= 7;
            out.push(if tag == 0 { b } else { b | 0x80 });
            if tag == 0 {
                break;
            }
        }
        let mut len = bytes.len() as u64;
        loop {
            let b = (len & 0x7f) as u8;
            len >>= 7;
            out.push(if len == 0 { b } else { b | 0x80 });
            if len == 0 {
                break;
            }
        }
        out.extend(bytes);
        out
    }

    /// A CRX3 file for `zip` signed by one key (`sign` makes the signature of the message), as Chrome's tools write it.
    fn package(key_spki: &[u8], zip: &[u8], sign: impl Fn(&[u8]) -> Vec<u8>, proof_field: u64) -> Vec<u8> {
        let digest = ring::digest::digest(&ring::digest::SHA256, key_spki);
        let signed = field(1, &digest.as_ref()[..16]);
        let mut message = b"CRX3 SignedData\0".to_vec();
        message.extend((signed.len() as u32).to_le_bytes());
        message.extend(&signed);
        message.extend(zip);
        let mut proof = field(1, key_spki);
        proof.extend(field(2, &sign(&message)));
        let mut header = field(proof_field, &proof);
        header.extend(field(10000, &signed));
        let mut out = b"Cr24".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend((header.len() as u32).to_le_bytes());
        out.extend(header);
        out.extend(zip);
        out
    }

    pub(crate) fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, text) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(text.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn rsa_package(zip: &[u8]) -> (String, Vec<u8>) {
        let pkcs8 = crate::extensions::base64_decode(RSA_TEST_KEY).unwrap();
        let pair = RsaKeyPair::from_pkcs8(&pkcs8).unwrap();
        let key = spki(&[&[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01][..], &[0x05, 0x00]].concat(), pair.public().as_ref());
        let crx = package(&key, zip, |m| {
            let mut sig = vec![0; pair.public().modulus_len()];
            pair.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), m, &mut sig).unwrap();
            sig
        }, 2);
        (id_from_bytes(&key), crx)
    }

    /// A new ECDSA signing key (PKCS#8), for tests that sign several packages with the same key.
    pub(crate) fn ecdsa_key() -> Vec<u8> {
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &SystemRandom::new()).unwrap().as_ref().to_vec()
    }

    pub(crate) fn ecdsa_package_with(pkcs8: &[u8], zip: &[u8]) -> (String, Vec<u8>) {
        let rng = SystemRandom::new();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8, &rng).unwrap();
        let params = [&[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01][..], &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]].concat();
        let key = spki(&params, pair.public_key().as_ref());
        let crx = package(&key, zip, |m| pair.sign(&rng, m).unwrap().as_ref().to_vec(), 3);
        (id_from_bytes(&key), crx)
    }

    fn ecdsa_package(zip: &[u8]) -> (String, Vec<u8>) {
        ecdsa_package_with(&ecdsa_key(), zip)
    }

    #[test]
    fn crx_rsa_and_ecdsa_packages_verify_and_give_their_id() {
        let zip = zip_of(&[("manifest.json", "{}")]);
        for (id, crx) in [rsa_package(&zip), ecdsa_package(&zip)] {
            let parsed = parse(&crx, &id).unwrap();
            assert_eq!(parsed.id, id);
            assert_eq!(parsed.zip, zip);
            assert_eq!(id_from_bytes(&parsed.key), id, "the key is the one the id comes from");
        }
    }

    #[test]
    fn crx_refuses_a_tampered_or_wrong_package() {
        let zip = zip_of(&[("manifest.json", "{}")]);
        let (id, crx) = rsa_package(&zip);
        let mut changed = crx.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert!(parse(&changed, &id).unwrap_err().contains("no valid signature"), "a changed zip");
        assert!(parse(&crx, "pppppppppppppppppppppppppppppppp").unwrap_err().contains("not pppp"), "another id");
        assert!(parse(b"<html>error</html>", &id).unwrap_err().contains("Cr24"));
        assert!(parse(&crx[..20], &id).is_err(), "cut short");
        let mut v2 = crx.clone();
        v2[4] = 2;
        assert!(parse(&v2, &id).unwrap_err().contains("CRX3"));
    }

    #[test]
    fn crx_unpack_writes_files_and_refuses_paths_that_leave_the_folder() {
        let dest = std::env::temp_dir().join(format!("browsah-crx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        unpack(&zip_of(&[("manifest.json", "{}"), ("a/b.js", "x"), ("_metadata/verified_contents.json", "skip")]), &dest).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("a/b.js")).unwrap(), "x");
        assert!(!dest.join("_metadata").exists());
        let evil = std::env::temp_dir().join(format!("browsah-crx-evil-{}", std::process::id()));
        assert!(unpack(&zip_of(&[("../escape.txt", "x")]), &evil).is_err());
        assert!(!evil.parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn crx_add_key_keeps_the_manifest_text_and_an_existing_key() {
        let out = add_key("{\n  \"name\": \"x\"\n}\n", &[1, 2, 3]).unwrap();
        assert_eq!(out, "{\n  \"key\": \"AQID\",\n  \"name\": \"x\"\n}\n");
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
        let keyed = "{\"key\":\"abc\",\"name\":\"x\"}";
        assert_eq!(add_key(keyed, &[1, 2, 3]).unwrap(), keyed);
        let with_comment = add_key("\u{feff}{ /* c */ \"name\": \"x\" }", &[1]).unwrap();
        assert!(with_comment.starts_with("{\n  \"key\": \"AQ==\","));
        assert!(add_key("nope", &[1]).is_err());
    }
}
