use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

pub const MAX_INTAKE_IMAGE_BYTES: usize = 1024 * 1024;
pub const MAX_INTAKE_IMAGES: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IntakeContext {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub images: Vec<IntakeImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IntakeImage {
    pub data_url: String,
    #[serde(default)]
    pub caption: String,
}

impl IntakeImage {
    pub fn image_parts(&self) -> Result<(&str, &str), String> {
        let (mime, data) = if let Some(data) = self.data_url.strip_prefix("data:image/png;base64,")
        {
            ("image/png", data)
        } else if let Some(data) = self.data_url.strip_prefix("data:image/jpeg;base64,") {
            ("image/jpeg", data)
        } else {
            return Err("Screenshots must be PNG or JPEG data images".into());
        };
        if data.len() > MAX_INTAKE_IMAGE_BYTES.div_ceil(3) * 4 {
            return Err("Each screenshot must be at most 1 MB".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Invalid screenshot encoding")?;
        if bytes.len() > MAX_INTAKE_IMAGE_BYTES || bytes.is_empty() {
            return Err("Each screenshot must be at most 1 MB".into());
        }
        let signature_valid = match mime {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            _ => bytes.starts_with(b"\xff\xd8\xff") && bytes.ends_with(b"\xff\xd9"),
        };
        if !signature_valid {
            return Err("Screenshot bytes do not match their image type".into());
        }
        Ok((mime, data))
    }
}

impl IntakeContext {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.len() > 40_000 {
            return Err("Task context exceeds 40 KB".into());
        }
        if self.images.len() > MAX_INTAKE_IMAGES {
            return Err("Paste at most three screenshots".into());
        }
        for image in &self.images {
            if image.caption.len() > 2_000 {
                return Err("Screenshot caption exceeds 2 KB".into());
            }
            image.image_parts()?;
        }
        Ok(())
    }
}
