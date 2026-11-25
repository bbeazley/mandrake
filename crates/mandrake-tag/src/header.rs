//! Myth II archive and tag headers.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::borrow::Cow;
use std::io::{Read, Write};

pub fn decode_c_string(bytes: &[u8]) -> String {
    mandrake_common::mac_roman::decode_c(bytes)
}

fn decode_c_cow(bytes: &[u8]) -> Cow<'_, str> {
    let slice = crate::parse::until_nul(bytes);
    if slice.is_ascii() {
        Cow::Borrowed(std::str::from_utf8(slice).expect("ASCII is UTF-8"))
    } else {
        Cow::Owned(mandrake_common::mac_roman::decode(slice))
    }
}

/// Helper to encode a string into a fixed-size null-terminated byte array.
pub fn encode_c_string<const N: usize>(s: &str) -> [u8; N] {
    let mut arr = [0u8; N];
    let bytes = s.as_bytes();
    let copy_len = bytes.len().min(N.saturating_sub(1));
    arr[..copy_len].copy_from_slice(&bytes[..copy_len]);
    arr
}

mod serde_array_64 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    pub fn serialize<S>(data: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        data.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let slice: Vec<u8> = Vec::deserialize(deserializer)?;
        if slice.len() != 64 {
            return Err(D::Error::custom(format!(
                "expected 64 bytes, got {}",
                slice.len()
            )));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&slice);
        Ok(arr)
    }
}

/// Monolithic archive header (128 bytes, big-endian).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SBMonoHeader {
    pub r#type: i16,
    pub version: u16,
    pub name: [u8; 32],
    #[serde(with = "serde_array_64")]
    pub description: [u8; 64],
    pub entry_tag_count: u16,
    pub tag_list_count: u16,
    pub checksum: [u8; 4],
    pub flags: u32,
    pub size: u32,
    pub header_checksum: [u8; 4],
    pub padding: [u8; 4],
    pub signature: FourCC,
}

impl SBMonoHeader {
    pub const SIZE: usize = 128;
    pub const SIG_DNG2: FourCC = FourCC(*b"dng2");
    pub const SIG_MTH2: FourCC = FourCC(*b"mth2");

    pub fn name_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.name)
    }

    pub fn description_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.description)
    }

    /// Read an `SBMonoHeader` from a byte reader.
    pub fn read<R: Read>(reader: &mut R) -> Result<Self, M2Error> {
        let r#type = reader.read_i16::<BigEndian>()?;
        let version = reader.read_u16::<BigEndian>()?;

        let mut name = [0u8; 32];
        reader.read_exact(&mut name)?;

        let mut description = [0u8; 64];
        reader.read_exact(&mut description)?;

        let entry_tag_count = reader.read_u16::<BigEndian>()?;
        let tag_list_count = reader.read_u16::<BigEndian>()?;

        let mut checksum = [0u8; 4];
        reader.read_exact(&mut checksum)?;

        let flags = reader.read_u32::<BigEndian>()?;
        let size = reader.read_u32::<BigEndian>()?;

        let mut header_checksum = [0u8; 4];
        reader.read_exact(&mut header_checksum)?;

        let mut padding = [0u8; 4];
        reader.read_exact(&mut padding)?;

        let mut signature_bytes = [0u8; 4];
        reader.read_exact(&mut signature_bytes)?;
        let signature = FourCC::new(signature_bytes);

        Ok(Self {
            r#type,
            version,
            name,
            description,
            entry_tag_count,
            tag_list_count,
            checksum,
            flags,
            size,
            header_checksum,
            padding,
            signature,
        })
    }

    /// Write an `SBMonoHeader` to a byte writer.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), M2Error> {
        writer.write_i16::<BigEndian>(self.r#type)?;
        writer.write_u16::<BigEndian>(self.version)?;
        writer.write_all(&self.name)?;
        writer.write_all(&self.description)?;
        writer.write_u16::<BigEndian>(self.entry_tag_count)?;
        writer.write_u16::<BigEndian>(self.tag_list_count)?;
        writer.write_all(&self.checksum)?;
        writer.write_u32::<BigEndian>(self.flags)?;
        writer.write_u32::<BigEndian>(self.size)?;
        writer.write_all(&self.header_checksum)?;
        writer.write_all(&self.padding)?;
        writer.write_all(self.signature.as_bytes())?;
        Ok(())
    }
}

/// Tag header describing an individual asset entry (64 bytes, big-endian).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SBTagHeader {
    pub identifier: i16,
    pub flags: i8,
    pub tag_type_idx: i8,
    pub name: [u8; 32],
    pub tag_type: FourCC,
    pub tag_id: FourCC,
    pub tag_data_offset: i32,
    pub tag_data_size: i32,
    pub user_data: u32,
    pub version: i16,
    pub destination: i8,
    pub owner_index: i8,
    pub signature: FourCC,
}

impl SBTagHeader {
    pub const SIZE: usize = 64;
    pub const SIGNATURE: FourCC = FourCC(*b"mth2");

    pub fn name_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.name)
    }

    /// Read an `SBTagHeader` from a byte reader.
    pub fn read<R: Read>(reader: &mut R) -> Result<Self, M2Error> {
        let identifier = reader.read_i16::<BigEndian>()?;
        let flags = reader.read_i8()?;
        let tag_type_idx = reader.read_i8()?;

        let mut name = [0u8; 32];
        reader.read_exact(&mut name)?;

        let mut type_bytes = [0u8; 4];
        reader.read_exact(&mut type_bytes)?;
        let tag_type = FourCC::new(type_bytes);

        let mut id_bytes = [0u8; 4];
        reader.read_exact(&mut id_bytes)?;
        let tag_id = FourCC::new(id_bytes);

        let tag_data_offset = reader.read_i32::<BigEndian>()?;
        let tag_data_size = reader.read_i32::<BigEndian>()?;
        let user_data = reader.read_u32::<BigEndian>()?;
        let version = reader.read_i16::<BigEndian>()?;
        let destination = reader.read_i8()?;
        let owner_index = reader.read_i8()?;

        let mut sig_bytes = [0u8; 4];
        reader.read_exact(&mut sig_bytes)?;
        let signature = FourCC::new(sig_bytes);

        Ok(Self {
            identifier,
            flags,
            tag_type_idx,
            name,
            tag_type,
            tag_id,
            tag_data_offset,
            tag_data_size,
            user_data,
            version,
            destination,
            owner_index,
            signature,
        })
    }

    /// Write an `SBTagHeader` to a byte writer.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), M2Error> {
        writer.write_i16::<BigEndian>(self.identifier)?;
        writer.write_i8(self.flags)?;
        writer.write_i8(self.tag_type_idx)?;
        writer.write_all(&self.name)?;
        writer.write_all(self.tag_type.as_bytes())?;
        writer.write_all(self.tag_id.as_bytes())?;
        writer.write_i32::<BigEndian>(self.tag_data_offset)?;
        writer.write_i32::<BigEndian>(self.tag_data_size)?;
        writer.write_u32::<BigEndian>(self.user_data)?;
        writer.write_i16::<BigEndian>(self.version)?;
        writer.write_i8(self.destination)?;
        writer.write_i8(self.owner_index)?;
        writer.write_all(self.signature.as_bytes())?;
        Ok(())
    }
}

/// Mission / entry tag description (112 bytes, big-endian).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SBEntryTag {
    pub tag_id: FourCC,
    pub reserved1: [u8; 12],
    pub internal_name: [u8; 32],
    pub display_name: [u8; 32],
    pub description: [u8; 32],
}

impl SBEntryTag {
    pub const SIZE: usize = 112;

    pub fn internal_name_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.internal_name)
    }

    pub fn display_name_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.display_name)
    }

    pub fn description_as_str(&self) -> Cow<'_, str> {
        decode_c_cow(&self.description)
    }

    pub fn read<R: Read>(reader: &mut R) -> Result<Self, M2Error> {
        let mut id_bytes = [0u8; 4];
        reader.read_exact(&mut id_bytes)?;
        let tag_id = FourCC::new(id_bytes);

        let mut reserved1 = [0u8; 12];
        reader.read_exact(&mut reserved1)?;

        let mut internal_name = [0u8; 32];
        reader.read_exact(&mut internal_name)?;

        let mut display_name = [0u8; 32];
        reader.read_exact(&mut display_name)?;

        let mut description = [0u8; 32];
        reader.read_exact(&mut description)?;

        Ok(Self {
            tag_id,
            reserved1,
            internal_name,
            display_name,
            description,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), M2Error> {
        writer.write_all(self.tag_id.as_bytes())?;
        writer.write_all(&self.reserved1)?;
        writer.write_all(&self.internal_name)?;
        writer.write_all(&self.display_name)?;
        writer.write_all(&self.description)?;
        Ok(())
    }
}
