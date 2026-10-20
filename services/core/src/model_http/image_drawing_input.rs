//! Drawing-only upload sidecar. Marketplace arguments contain metadata, while
//! Core freezes verified bytes privately beside that original argument digest.
use super::super::image_provider::{ImageMediaType, ImageMultipartInput, ImageUpload, dimensions};
use super::{DrawingInput, Error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const MAX_REFERENCES: usize = 8;
pub(crate) const MAX_REFERENCE_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_TOTAL_REFERENCE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_PREVIEW_BODY: usize = 24 * 1024 * 1024 + 64 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceImage {
    pub media_type: String,
    pub sha256: String,
    pub byte_length: u64,
}
/// Never Debug: encoded user image bytes are private operation data.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingUpload {
    pub media_type: String,
    pub sha256: String,
    pub byte_length: u64,
    pub data_base64: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct DrawingPreviewInput {
    #[serde(flatten)]
    pub settings: DrawingInput,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reference_images: Vec<ReferenceImage>,
}
impl std::ops::Deref for DrawingPreviewInput {
    type Target = DrawingInput;
    fn deref(&self) -> &DrawingInput {
        &self.settings
    }
}
impl From<DrawingInput> for DrawingPreviewInput {
    fn from(settings: DrawingInput) -> Self {
        Self {
            settings,
            reference_images: vec![],
        }
    }
}
// Reviewed additive reference-image contract. The binding comes from the
// persisted Core quote, never the browser's claimed tool/version metadata.
pub(crate) fn authorize_references(raw: &Value, version: &str, schema: &str) -> Result<(), Error> {
    let parsed = parse_arguments(raw)?;
    if !parsed.reference_images.is_empty()
        && (version != "ca3a1996-723d-5695-b29e-40e3bef58644"
            || schema != "abd718715e097d729e362afd0f5cdf19ae664886a878902e1c38ac31a2c2964e")
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}
fn media_type(value: &str) -> Result<ImageMediaType, Error> {
    match value {
        "image/png" => Ok(ImageMediaType::Png),
        "image/jpeg" => Ok(ImageMediaType::Jpeg),
        "image/webp" => Ok(ImageMediaType::Webp),
        _ => Err(Error::Invalid),
    }
}
fn valid_reference(reference: &ReferenceImage) -> bool {
    reference.byte_length > 0
        && reference.byte_length <= MAX_REFERENCE_BYTES as u64
        && reference.sha256.len() == 64
        && reference
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && media_type(&reference.media_type).is_ok()
}
pub(crate) fn parse_arguments(raw: &Value) -> Result<DrawingPreviewInput, Error> {
    let mut object = raw.as_object().ok_or(Error::Invalid)?.clone();
    let references: Vec<ReferenceImage> = match object.remove("reference_images") {
        Some(v) => serde_json::from_value(v).map_err(|_| Error::Invalid)?,
        None => vec![],
    };
    if references.len() > MAX_REFERENCES
        || references.iter().any(|r| !valid_reference(r))
        || references
            .iter()
            .try_fold(0u64, |n, r| n.checked_add(r.byte_length))
            .is_none_or(|n| n > MAX_TOTAL_REFERENCE_BYTES as u64)
    {
        return Err(Error::Invalid);
    }
    let settings: DrawingInput =
        serde_json::from_value(Value::Object(object)).map_err(|_| Error::Invalid)?;
    Ok(DrawingPreviewInput {
        settings,
        reference_images: references,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    kind: String,
    arguments: Value,
    uploads: Vec<DrawingUpload>,
}
/// Only this exact Core-created envelope may contain persisted upload bytes.
/// Legacy generation/native-Key rows keep their original flat representation.
pub(crate) fn arguments(raw: &Value) -> Result<Value, Error> {
    if raw.get("kind").and_then(Value::as_str) == Some("drawing_reference_v1") {
        let object = raw.as_object().ok_or(Error::Storage)?;
        if object.len() != 3 || !object.contains_key("uploads") {
            return Err(Error::Storage);
        }
        Ok(object.get("arguments").ok_or(Error::Storage)?.clone())
    } else {
        Ok(raw.clone())
    }
}
pub(crate) fn freeze(raw: Value, uploads: Vec<DrawingUpload>) -> Result<Value, Error> {
    let parsed = parse_arguments(&raw)?;
    decode_uploads(&parsed.reference_images, &uploads)?;
    if uploads.is_empty() {
        Ok(raw)
    } else {
        Ok(json!({"kind":"drawing_reference_v1","arguments":raw,"uploads":uploads}))
    }
}
pub(crate) fn restore(raw: &Value) -> Result<Option<ImageMultipartInput>, Error> {
    if raw.get("kind").and_then(Value::as_str) == Some("drawing_reference_v1") {
        let stored: Stored = serde_json::from_value(raw.clone()).map_err(|_| Error::Storage)?;
        if stored.kind != "drawing_reference_v1" {
            return Err(Error::Storage);
        }
        let parsed = parse_arguments(&stored.arguments).map_err(|_| Error::Storage)?;
        decode_uploads(&parsed.reference_images, &stored.uploads).map_err(|_| Error::Storage)
    } else {
        let parsed = parse_arguments(raw).map_err(|_| Error::Storage)?;
        if !parsed.reference_images.is_empty() {
            return Err(Error::Storage);
        }
        Ok(None)
    }
}
fn decode_uploads(
    references: &[ReferenceImage],
    uploads: &[DrawingUpload],
) -> Result<Option<ImageMultipartInput>, Error> {
    if references.len() != uploads.len() || uploads.len() > MAX_REFERENCES {
        return Err(Error::Invalid);
    }
    let mut images = Vec::new();
    let mut total = 0usize;
    for (reference, upload) in references.iter().zip(uploads) {
        if !valid_reference(reference)
            || reference.media_type != upload.media_type
            || reference.sha256 != upload.sha256
            || reference.byte_length != upload.byte_length
        {
            return Err(Error::Invalid);
        }
        let expected = usize::try_from(reference.byte_length).map_err(|_| Error::Invalid)?;
        if upload.data_base64.len() != expected.div_ceil(3) * 4 {
            return Err(Error::Invalid);
        }
        let bytes = STANDARD
            .decode(&upload.data_base64)
            .map_err(|_| Error::Invalid)?;
        total = total
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_TOTAL_REFERENCE_BYTES)
            .ok_or(Error::Invalid)?;
        if bytes.len() != expected
            || STANDARD.encode(&bytes) != upload.data_base64
            || format!("{:x}", Sha256::digest(&bytes)) != reference.sha256
        {
            return Err(Error::Invalid);
        }
        let image = ImageUpload {
            bytes,
            media_type: media_type(&reference.media_type)?,
        };
        dimensions(&image).map_err(|_| Error::Invalid)?;
        images.push(image);
    }
    if images.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ImageMultipartInput { images, mask: None }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> DrawingUpload {
        let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==";
        let bytes = STANDARD.decode(data).unwrap();
        DrawingUpload {
            media_type: "image/png".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            byte_length: bytes.len() as u64,
            data_base64: data.into(),
        }
    }
    fn args(upload: &DrawingUpload) -> Value {
        json!({"prompt":"edit synthetic reference","model":"gpt-image-1","group":"allowed","n":1,"reference_images":[{"media_type":upload.media_type,"sha256":upload.sha256,"byte_length":upload.byte_length}]})
    }
    #[test]
    fn drawing_uploads_freeze_exact_content_but_public_projection_has_no_bytes() {
        let image = png();
        let raw = args(&image);
        let stored = freeze(raw.clone(), vec![image.clone()]).unwrap();
        assert_eq!(arguments(&stored).unwrap(), raw);
        let parsed = parse_arguments(&arguments(&stored).unwrap()).unwrap();
        let public = serde_json::to_string(&parsed).unwrap();
        assert!(!public.contains("data_base64"));
        assert!(!public.contains(&image.data_base64));
        assert!(public.contains(&image.sha256));
        let restored = restore(&stored).unwrap().unwrap();
        assert_eq!(restored.images.len(), 1);
        assert_eq!(
            restored.images[0].bytes,
            STANDARD.decode(&image.data_base64).unwrap()
        );
        assert!(restored.mask.is_none());
        let mut corrupt = stored.clone();
        corrupt["uploads"][0]["sha256"] = json!("a".repeat(64));
        assert!(matches!(restore(&corrupt), Err(Error::Storage)));
    }
    #[test]
    fn drawing_reference_metadata_cannot_substitute_urls_hashes_lengths_or_mime() {
        let image = png();
        let raw = args(&image);
        assert!(freeze(raw.clone(), vec![]).is_err());
        for field in ["sha256", "media_type", "byte_length"] {
            let mut altered = raw.clone();
            altered["reference_images"][0][field] = match field {
                "sha256" => json!("b".repeat(64)),
                "media_type" => json!("image/jpeg"),
                _ => json!(image.byte_length + 1),
            };
            assert!(freeze(altered, vec![image.clone()]).is_err());
        }
        let mut unknown = raw.clone();
        unknown["reference_images"][0]["url"] = json!("https://example.invalid/private");
        assert!(parse_arguments(&unknown).is_err());
        let mut raw = raw;
        raw["reference_images"][0]["byte_length"] = json!(MAX_REFERENCE_BYTES + 1);
        assert!(parse_arguments(&raw).is_err());
        let mut impostor = image.clone();
        impostor.media_type = "image/jpeg".into();
        let mut raw = args(&impostor);
        assert!(freeze(raw.clone(), vec![impostor]).is_err());
        raw["reference_images"] = Value::Array(vec![
            args(&image)["reference_images"][0].clone();
            MAX_REFERENCES + 1
        ]);
        assert!(parse_arguments(&raw).is_err());
    }
    #[test]
    fn drawing_generation_keeps_original_digest_shape_and_rejects_hidden_sidecar() {
        let raw = json!({"prompt":"generate","model":"gpt-image-1","group":"allowed"});
        assert_eq!(freeze(raw.clone(), vec![]).unwrap(), raw);
        assert!(restore(&raw).unwrap().is_none());
        assert!(freeze(raw.clone(), vec![png()]).is_err());
        let mut forged = raw;
        forged["uploads"] = json!([]);
        assert!(parse_arguments(&forged).is_err());
    }
}
