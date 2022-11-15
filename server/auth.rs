use enum_dispatch::enum_dispatch;
use jwt::algorithm::openssl::PKeyWithDigest;
use jwt::{Header, Token, VerifyWithKey};
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Public};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Read};

#[enum_dispatch]
#[derive(Clone, Debug)]
pub enum Auth {
    JWT(JWT),
}

#[derive(Deserialize, Debug, Clone)]
pub struct JWT {
    pub iat: u64,
    pub exp: u64,
    pub pubkey: Option<String>,
    pub partner: Option<String>,
}

fn authorization_header_auth(
    public_key: PKey<Public>,
    header: String,
) -> Result<Auth, Box<dyn std::error::Error + Send + Sync>> {
    let public_key = PKeyWithDigest {
        digest: MessageDigest::sha256(),
        key: public_key,
    };
    log::info!("Header: {}", header.clone());
    let mut header_parts = header.split_whitespace();
    if Some("Bearer").ne(&header_parts.next()) {
        return Err("Authorization header must start with 'Bearer '!".into());
    }
    let token = header_parts.next();
    if let Some(_) = header_parts.next() {
        return Err("There must be no extra characters after Bearer token!".into());
    }
    match token {
        Some(token) => {
            let token: Token<Header, JWT, _> = token.verify_with_key(&public_key)?;
            Ok(Auth::JWT(token.claims().clone()))
        }
        _ => Err("Bearer token is missing!".into()),
    }
}

pub fn authenticate(
    public_key: PKey<Public>,
    header: Option<String>,
) -> Result<Auth, Box<dyn std::error::Error + Send + Sync>> {
    match header {
        Some(header) => Ok(authorization_header_auth(public_key, header)?),
        _ => Err("Request is not authenticated!".into()),
    }
}

pub fn load_public_key(
    path: String,
) -> Result<PKey<Public>, Box<dyn std::error::Error + Send + Sync>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();

    reader.read_to_end(&mut buffer)?;

    Ok(PKey::public_key_from_pem(&buffer)?)
}
