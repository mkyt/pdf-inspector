//! Standard security handler (ISO 32000-2 7.6.4): RC4 and AES, R2–R6.

use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::filters::Decoder;
use crate::object::{Dict, DictExt, Name, ObjId, Object};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptMethod {
    None,
    Rc4,
    AesV2,
    AesV3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthResult {
    User,
    Owner,
}

#[derive(Clone, Debug)]
pub struct EncryptInfo {
    pub filter: Name,
    pub v: i64,
    pub r: i64,
    pub key_bits: usize,
    pub permissions: i32,
    pub encrypt_metadata: bool,
    pub stream_method: CryptMethod,
    pub string_method: CryptMethod,
}

#[derive(Clone, Debug)]
pub struct SecurityHandler {
    pub info: EncryptInfo,
    /// File key once a password has been authenticated.
    key: Option<Vec<u8>>,
    pub auth: Option<AuthResult>,
    o: Vec<u8>,
    u: Vec<u8>,
    oe: Vec<u8>,
    ue: Vec<u8>,
    id0: Vec<u8>,
    /// Named crypt filters from `/CF`.
    filters: Vec<(Name, CryptMethod)>,
}

fn bytes_of(d: &Dict, key: &str) -> Vec<u8> {
    d.get_key(key)
        .and_then(Object::as_string)
        .map(|s| s.bytes.clone())
        .unwrap_or_default()
}

fn method_of(cf: &Dict) -> CryptMethod {
    match cf.get_name("CFM").map(|n| n.as_bytes()) {
        Some(b"V2") => CryptMethod::Rc4,
        Some(b"AESV2") => CryptMethod::AesV2,
        Some(b"AESV3") => CryptMethod::AesV3,
        _ => CryptMethod::None,
    }
}

impl SecurityHandler {
    /// `encrypt` must have its direct values resolved.
    pub fn new(encrypt: &Dict, id0: &[u8]) -> Result<Self, String> {
        let filter = encrypt
            .get_name("Filter")
            .cloned()
            .unwrap_or_else(|| Name::from("Standard"));
        if filter != "Standard" {
            return Err(format!("unsupported security handler {filter:?}"));
        }
        let v = encrypt.get_i64("V").unwrap_or(0);
        let r = encrypt.get_i64("R").unwrap_or(2);
        let mut filters = Vec::new();
        if let Some(Object::Dict(cf)) = encrypt.get_key("CF") {
            for (name, node) in cf {
                if let Some(d) = node.obj.as_dict() {
                    filters.push((name.clone(), method_of(d)));
                }
            }
        }
        let lookup = |key: &str| -> CryptMethod {
            match encrypt.get_name(key) {
                None => CryptMethod::None,
                Some(n) if n == "Identity" => CryptMethod::None,
                Some(n) => filters
                    .iter()
                    .find(|(f, _)| f == n)
                    .map_or(CryptMethod::None, |(_, m)| *m),
            }
        };
        let (stream_method, string_method) = match v {
            1 | 2 => (CryptMethod::Rc4, CryptMethod::Rc4),
            4 | 5 => (lookup("StmF"), lookup("StrF")),
            _ => return Err(format!("unsupported encryption /V {v}")),
        };
        let key_bits = match v {
            1 => 40,
            5 => 256,
            _ => {
                let l = encrypt.get_i64("Length").unwrap_or(40);
                // Some writers store bytes instead of bits.
                (if l <= 16 { l * 8 } else { l }).clamp(40, 128) as usize
            }
        };
        let key_bits = if v == 4 && stream_method == CryptMethod::AesV2 {
            128
        } else {
            key_bits
        };
        Ok(Self {
            info: EncryptInfo {
                filter,
                v,
                r,
                key_bits,
                permissions: encrypt.get_i64("P").unwrap_or(0) as i32,
                encrypt_metadata: !matches!(
                    encrypt.get_key("EncryptMetadata"),
                    Some(Object::Bool(false))
                ),
                stream_method,
                string_method,
            },
            key: None,
            auth: None,
            o: bytes_of(encrypt, "O"),
            u: bytes_of(encrypt, "U"),
            oe: bytes_of(encrypt, "OE"),
            ue: bytes_of(encrypt, "UE"),
            id0: id0.to_vec(),
            filters,
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.key.is_some()
    }

    pub fn file_key(&self) -> Option<&[u8]> {
        self.key.as_deref()
    }

    /// Try `password` as user then owner password.
    pub fn authenticate(&mut self, password: &[u8]) -> Option<AuthResult> {
        let res = if self.info.r >= 5 {
            self.auth_r56(password)
        } else {
            self.auth_r24(password)
        };
        if let Some((key, who)) = res {
            self.key = Some(key);
            self.auth = Some(who);
        }
        self.auth
    }

    fn key_len(&self) -> usize {
        if self.info.r == 2 {
            5
        } else {
            self.info.key_bits / 8
        }
    }

    /// Algorithm 2.
    fn compute_key_r24(&self, password: &[u8]) -> Vec<u8> {
        let mut h = Md5::new();
        h.update(padded(password));
        h.update(&self.o[..self.o.len().min(32)]);
        h.update(self.info.permissions.to_le_bytes());
        h.update(&self.id0);
        if self.info.r >= 4 && !self.info.encrypt_metadata {
            h.update([0xff; 4]);
        }
        let n = self.key_len();
        let mut key = h.finalize().to_vec();
        if self.info.r >= 3 {
            for _ in 0..50 {
                key = Md5::digest(&key[..n]).to_vec();
            }
        }
        key.truncate(n);
        key
    }

    /// Algorithms 4/5 output for a candidate key.
    fn u_value(&self, key: &[u8]) -> Vec<u8> {
        if self.info.r == 2 {
            return rc4(key, &PAD);
        }
        let mut h = Md5::new();
        h.update(PAD);
        h.update(&self.id0);
        let mut x = h.finalize().to_vec();
        for i in 0..20u8 {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            x = rc4(&k, &x);
        }
        x
    }

    fn check_user_r24(&self, password: &[u8]) -> Option<Vec<u8>> {
        let key = self.compute_key_r24(password);
        let u = self.u_value(&key);
        let n = if self.info.r == 2 { 32 } else { 16 };
        (self.u.len() >= n && u[..n] == self.u[..n]).then_some(key)
    }

    fn auth_r24(&self, password: &[u8]) -> Option<(Vec<u8>, AuthResult)> {
        if let Some(k) = self.check_user_r24(password) {
            return Some((k, AuthResult::User));
        }
        // Algorithm 7: recover the user password from /O.
        let mut digest = Md5::digest(padded(password)).to_vec();
        if self.info.r >= 3 {
            for _ in 0..50 {
                digest = Md5::digest(&digest).to_vec();
            }
        }
        let n = self.key_len();
        let okey = &digest[..n];
        let mut user = self.o[..self.o.len().min(32)].to_vec();
        if self.info.r == 2 {
            user = rc4(okey, &user);
        } else {
            for i in (0..20u8).rev() {
                let k: Vec<u8> = okey.iter().map(|b| b ^ i).collect();
                user = rc4(&k, &user);
            }
        }
        self.check_user_r24(&user).map(|k| (k, AuthResult::Owner))
    }

    fn auth_r56(&self, password: &[u8]) -> Option<(Vec<u8>, AuthResult)> {
        let pw = &password[..password.len().min(127)];
        if self.u.len() < 48 || self.o.len() < 48 {
            return None;
        }
        let r = self.info.r;
        let (u_hash, u_vsalt, u_ksalt) = (&self.u[..32], &self.u[32..40], &self.u[40..48]);
        let (o_hash, o_vsalt, o_ksalt) = (&self.o[..32], &self.o[32..40], &self.o[40..48]);
        let udata = &self.u[..48];
        if hash_r56(r, pw, o_vsalt, udata) == o_hash {
            let k = hash_r56(r, pw, o_ksalt, udata);
            return Some((aes256_cbc_nopad_decrypt(&k, &self.oe), AuthResult::Owner));
        }
        if hash_r56(r, pw, u_vsalt, &[]) == u_hash {
            let k = hash_r56(r, pw, u_ksalt, &[]);
            return Some((aes256_cbc_nopad_decrypt(&k, &self.ue), AuthResult::User));
        }
        None
    }

    /// Algorithm 1: per-object key.
    fn object_key(&self, id: ObjId, method: CryptMethod) -> Option<Vec<u8>> {
        let key = self.key.as_ref()?;
        if method == CryptMethod::AesV3 {
            return Some(key.clone());
        }
        let mut h = Md5::new();
        h.update(key);
        h.update(&id.num.to_le_bytes()[..3]);
        h.update(id.generation.to_le_bytes());
        if method == CryptMethod::AesV2 {
            h.update(b"sAlT");
        }
        let mut k = h.finalize().to_vec();
        k.truncate((key.len() + 5).min(16));
        Some(k)
    }

    pub fn method_for_filter(&self, name: &Name) -> CryptMethod {
        if name == "Identity" {
            return CryptMethod::None;
        }
        self.filters
            .iter()
            .find(|(f, _)| f == name)
            .map_or(self.info.stream_method, |(_, m)| *m)
    }

    pub fn decrypt_string(&self, id: ObjId, data: &[u8]) -> Option<Vec<u8>> {
        let m = self.info.string_method;
        let key = self.object_key(id, m)?;
        Some(match m {
            CryptMethod::None => data.to_vec(),
            CryptMethod::Rc4 => rc4(&key, data),
            CryptMethod::AesV2 | CryptMethod::AesV3 => aes_cbc_decrypt(&key, data),
        })
    }

    pub fn encrypt_string_method(&self) -> CryptMethod {
        self.info.string_method
    }

    /// Decoder that decrypts stream data of object `id` with `method`.
    pub fn stream_decoder(&self, id: ObjId, method: CryptMethod) -> Option<Box<dyn Decoder>> {
        let key = self.object_key(id, method)?;
        Some(match method {
            CryptMethod::None => return None,
            CryptMethod::Rc4 => Box::new(Rc4::new(&key)),
            CryptMethod::AesV2 | CryptMethod::AesV3 => Box::new(AesCbcDecoder::new(key)),
        })
    }
}

fn padded(password: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = password.len().min(32);
    out[..n].copy_from_slice(&password[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

/// Algorithm 2.A / 2.B hash.
fn hash_r56(r: i64, password: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = {
        let mut h = Sha256::new();
        h.update(password);
        h.update(salt);
        h.update(udata);
        h.finalize().to_vec()
    };
    if r == 5 {
        return k;
    }
    let mut i = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let e = aes128_cbc_nopad_encrypt(&k[..16], &k[16..32], &k1);
        let m = e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3;
        k = match m {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        i += 1;
        let last = u32::from(*e.last().unwrap_or(&0));
        if i >= 64 && last <= i - 32 {
            break;
        }
    }
    k.truncate(32);
    k
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut r = Rc4::new(key);
    let mut out = Vec::with_capacity(data.len());
    r.apply(data, &mut out);
    out
}

pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Self {
        let mut s = [0u8; 256];
        for (i, v) in s.iter_mut().enumerate() {
            *v = i as u8;
        }
        if !key.is_empty() {
            let mut j = 0u8;
            for i in 0..256 {
                j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
                s.swap(i, j as usize);
            }
        }
        Self { s, i: 0, j: 0 }
    }

    fn apply(&mut self, data: &[u8], out: &mut Vec<u8>) {
        for &b in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[self.i as usize]);
            self.s.swap(self.i as usize, self.j as usize);
            let k =
                self.s[(self.s[self.i as usize].wrapping_add(self.s[self.j as usize])) as usize];
            out.push(b ^ k);
        }
    }
}

impl Decoder for Rc4 {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        self.apply(input, out);
        Ok(())
    }

    fn finish(&mut self, _out: &mut Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

enum AnyAes {
    A128(Box<Aes128>),
    A256(Box<Aes256>),
}

impl AnyAes {
    fn new(key: &[u8]) -> Option<Self> {
        match key.len() {
            16 => Some(Self::A128(Box::new(Aes128::new(
                &Array::try_from(key).ok()?,
            )))),
            32 => Some(Self::A256(Box::new(Aes256::new(
                &Array::try_from(key).ok()?,
            )))),
            _ => None,
        }
    }

    fn decrypt(&self, block: &mut [u8; 16]) {
        let mut b = Array::from(*block);
        match self {
            Self::A128(c) => c.decrypt_block(&mut b),
            Self::A256(c) => c.decrypt_block(&mut b),
        }
        block.copy_from_slice(&b);
    }

    fn encrypt(&self, block: &mut [u8; 16]) {
        let mut b = Array::from(*block);
        match self {
            Self::A128(c) => c.encrypt_block(&mut b),
            Self::A256(c) => c.encrypt_block(&mut b),
        }
        block.copy_from_slice(&b);
    }
}

/// Streaming AES-CBC decryption with the IV in the first 16 bytes and
/// PKCS#7 padding (removed leniently).
pub struct AesCbcDecoder {
    key: Vec<u8>,
    cipher: Option<AnyAes>,
    iv: Option<[u8; 16]>,
    buf: Vec<u8>,
    /// Last decrypted block, held back until we know it is the final one.
    held: Option<[u8; 16]>,
}

impl AesCbcDecoder {
    pub fn new(key: Vec<u8>) -> Self {
        let cipher = AnyAes::new(&key);
        Self {
            key,
            cipher,
            iv: None,
            buf: Vec::new(),
            held: None,
        }
    }
}

impl Decoder for AesCbcDecoder {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        let Some(cipher) = &self.cipher else {
            return Err(format!("invalid AES key length {}", self.key.len()));
        };
        self.buf.extend_from_slice(input);
        let mut off = 0;
        if self.iv.is_none() {
            if self.buf.len() < 16 {
                return Ok(());
            }
            self.iv = Some(self.buf[..16].try_into().unwrap_or([0; 16]));
            off = 16;
        }
        let mut iv = self.iv.unwrap_or([0; 16]);
        while self.buf.len() - off >= 16 {
            let ct: [u8; 16] = self.buf[off..off + 16].try_into().unwrap_or([0; 16]);
            let mut b = ct;
            cipher.decrypt(&mut b);
            for (x, v) in b.iter_mut().zip(iv) {
                *x ^= v;
            }
            iv = ct;
            if let Some(h) = self.held.replace(b) {
                out.extend_from_slice(&h);
            }
            off += 16;
        }
        self.iv = Some(iv);
        self.buf.drain(..off);
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        if let Some(h) = self.held.take() {
            let pad = usize::from(h[15]);
            if (1..=16).contains(&pad) && h[16 - pad..].iter().all(|&b| usize::from(b) == pad) {
                out.extend_from_slice(&h[..16 - pad]);
            } else {
                out.extend_from_slice(&h);
            }
        }
        if !self.buf.is_empty() {
            return Err("AES data is not a multiple of the block size".into());
        }
        Ok(())
    }
}

fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut d = AesCbcDecoder::new(key.to_vec());
    let mut out = Vec::new();
    let _ = d.feed(data, &mut out);
    let _ = d.finish(&mut out);
    out
}

fn aes128_cbc_nopad_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    let Some(c) = AnyAes::new(key) else {
        return Vec::new();
    };
    let mut prev: [u8; 16] = iv.try_into().unwrap_or([0; 16]);
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut b: [u8; 16] = chunk.try_into().unwrap_or([0; 16]);
        for (x, v) in b.iter_mut().zip(prev) {
            *x ^= v;
        }
        c.encrypt(&mut b);
        out.extend_from_slice(&b);
        prev = b;
    }
    out
}

fn aes256_cbc_nopad_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let Some(c) = AnyAes::new(key) else {
        return Vec::new();
    };
    let mut prev = [0u8; 16];
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let ct: [u8; 16] = chunk.try_into().unwrap_or([0; 16]);
        let mut b = ct;
        c.decrypt(&mut b);
        for (x, v) in b.iter_mut().zip(prev) {
            *x ^= v;
        }
        out.extend_from_slice(&b);
        prev = ct;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_vector() {
        // RFC 6229 style known vector: key "Key", plaintext "Plaintext".
        assert_eq!(
            rc4(b"Key", b"Plaintext"),
            [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]
        );
    }

    #[test]
    fn aes_stream_roundtrip() {
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let mut msg = b"hello aes world!!".to_vec();
        let pad = 16 - msg.len() % 16;
        msg.extend(std::iter::repeat_n(pad as u8, pad));
        let mut data = iv.to_vec();
        data.extend(aes128_cbc_nopad_encrypt(&key, &iv, &msg));
        let mut d = AesCbcDecoder::new(key.to_vec());
        let mut out = Vec::new();
        for c in data.chunks(5) {
            d.feed(c, &mut out).unwrap();
        }
        d.finish(&mut out).unwrap();
        assert_eq!(out, b"hello aes world!!");
    }
}
