fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            ".ross.store.BlobInfo",
            "#[serde(default)]",
        )
        .type_attribute(
            ".ross.store.ManifestInfo",
            "#[serde(default)]",
        )
        .type_attribute(
            ".ross.store.TagInfo",
            "#[serde(default)]",
        )
        .field_attribute(
            ".ross.store.BlobInfo.created_at",
            "#[serde(skip)]",
        )
        .field_attribute(
            ".ross.store.BlobInfo.accessed_at",
            "#[serde(skip)]",
        )
        .field_attribute(
            ".ross.store.ManifestInfo.created_at",
            "#[serde(skip)]",
        )
        .field_attribute(
            ".ross.store.TagInfo.updated_at",
            "#[serde(skip)]",
        )
        .compile_protos(&["../proto/store.proto"], &["../proto"])?;
    Ok(())
}
