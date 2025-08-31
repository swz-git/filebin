use core::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ormlite::{Decode, Encode};
use rand::Rng;
use sqlx::Type;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slug([u8; 12]);

impl Slug {
    pub fn gen_random() -> Self {
        let mut rng = rand::rng();
        let random_data: [u8; 12] = rng.random();
        Self(random_data)
    }
}

impl std::str::FromStr for Slug {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let decoded = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| "Base64 decode failed")?;
        if decoded.len() != 12 {
            Err("Invalid length")
        } else {
            let mut slug = [0u8; 12];
            slug.copy_from_slice(&decoded);
            Ok(Self(slug))
        }
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out_buf = [0u8; 16];
        URL_SAFE_NO_PAD.encode_slice(self.0, &mut out_buf).unwrap();
        write!(f, "{}", String::from_utf8_lossy(&out_buf))
    }
}

impl fmt::Debug for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // print as hex for convenience
        write!(f, "Slug(")?;
        for b in &self.0 {
            write!(f, "{:02x}", b)?;
        }
        write!(f, ")")
    }
}

impl<DB: sqlx::Database> Type<DB> for Slug
where
    Vec<u8>: Type<DB>,
{
    fn type_info() -> <DB as sqlx::Database>::TypeInfo {
        <Vec<u8> as Type<DB>>::type_info()
    }
}

impl<'q, DB> Encode<'q, DB> for Slug
where
    DB: sqlx::Database,
    Vec<u8>: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        self.0.to_vec().encode_by_ref(buf)
    }
}

impl<'r, DB> Decode<'r, DB> for Slug
where
    DB: sqlx::Database,
    Vec<u8>: Decode<'r, DB>,
{
    fn decode(
        value: <DB as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        const SIZE: usize = 12;

        let vec = Vec::<u8>::decode(value)?;
        if vec.len() != SIZE {
            return Err(format!("Expected 16 bytes, got {}", vec.len()).into());
        }
        let mut arr = [0u8; SIZE];
        arr.copy_from_slice(&vec);
        Ok(Slug(arr))
    }
}
