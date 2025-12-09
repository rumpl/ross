use crate::error::RegistryError;

#[derive(Debug, Clone)]
pub struct ImageReference {
    pub registry: String,
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl ImageReference {
    pub fn parse(reference: &str) -> Result<Self, RegistryError> {
        let reference = reference.trim();

        let (reference, digest) = if let Some(idx) = reference.rfind('@') {
            let digest = reference[idx + 1..].to_string();
            let reference = &reference[..idx];
            (reference, Some(digest))
        } else {
            (reference, None)
        };

        let (reference, tag) = if digest.is_none() {
            if let Some(idx) = reference.rfind(':') {
                let potential_tag = &reference[idx + 1..];
                if !potential_tag.contains('/') {
                    (&reference[..idx], Some(potential_tag.to_string()))
                } else {
                    (reference, None)
                }
            } else {
                (reference, None)
            }
        } else {
            (reference, None)
        };

        let (registry, repository) = if reference.contains('/') {
            let first_slash = reference.find('/').unwrap();
            let first_part = &reference[..first_slash];

            if first_part.contains('.') || first_part.contains(':') || first_part == "localhost" {
                (
                    first_part.to_string(),
                    reference[first_slash + 1..].to_string(),
                )
            } else {
                ("registry-1.docker.io".to_string(), reference.to_string())
            }
        } else {
            (
                "registry-1.docker.io".to_string(),
                format!("library/{}", reference),
            )
        };

        Ok(Self {
            registry,
            repository,
            tag,
            digest,
        })
    }

    pub fn tag_or_default(&self) -> &str {
        self.tag.as_deref().unwrap_or("latest")
    }

    pub fn reference(&self) -> String {
        if let Some(digest) = &self.digest {
            digest.clone()
        } else {
            self.tag_or_default().to_string()
        }
    }

    pub fn full_name(&self) -> String {
        let tag = self.tag_or_default();
        if self.registry == "registry-1.docker.io" {
            if self.repository.starts_with("library/") {
                format!("{}:{}", &self.repository[8..], tag)
            } else {
                format!("{}:{}", self.repository, tag)
            }
        } else {
            format!("{}/{}:{}", self.registry, self.repository, tag)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let r = ImageReference::parse("nginx").unwrap();
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, None);
        assert_eq!(r.tag_or_default(), "latest");
    }

    #[test]
    fn test_parse_with_tag() {
        let r = ImageReference::parse("nginx:alpine").unwrap();
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, Some("alpine".to_string()));
    }

    #[test]
    fn test_parse_with_namespace() {
        let r = ImageReference::parse("myuser/myimage:v1").unwrap();
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "myuser/myimage");
        assert_eq!(r.tag, Some("v1".to_string()));
    }

    #[test]
    fn test_parse_custom_registry() {
        let r = ImageReference::parse("ghcr.io/owner/repo:latest").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "owner/repo");
        assert_eq!(r.tag, Some("latest".to_string()));
    }
}
