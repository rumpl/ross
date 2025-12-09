use crate::error::RegistryError;
use crate::reference::ImageReference;
use crate::types::*;
use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct RegistryClient {
    client: Client,
    tokens: Arc<RwLock<std::collections::HashMap<String, String>>>,
}

impl RegistryClient {
    pub fn new() -> Result<Self, RegistryError> {
        let client = Client::builder().user_agent("ross/0.1.0").build()?;

        Ok(Self {
            client,
            tokens: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    fn registry_url(&self, registry: &str) -> String {
        if registry.starts_with("localhost") || registry.contains("127.0.0.1") {
            format!("http://{}", registry)
        } else {
            format!("https://{}", registry)
        }
    }

    async fn get_token(&self, reference: &ImageReference) -> Result<Option<String>, RegistryError> {
        let key = format!("{}/{}", reference.registry, reference.repository);
        let tokens = self.tokens.read().await;
        Ok(tokens.get(&key).cloned())
    }

    async fn authenticate(
        &self,
        reference: &ImageReference,
        www_auth: &str,
    ) -> Result<String, RegistryError> {
        let realm = extract_auth_param(www_auth, "realm")
            .ok_or_else(|| RegistryError::AuthFailed("no realm in www-authenticate".to_string()))?;
        let service = extract_auth_param(www_auth, "service");
        let scope = extract_auth_param(www_auth, "scope")
            .unwrap_or_else(|| format!("repository:{}:pull", reference.repository));

        let mut url = realm.to_string();
        url.push_str("?scope=");
        url.push_str(&scope);
        if let Some(svc) = service {
            url.push_str("&service=");
            url.push_str(&svc);
        }

        tracing::debug!("Authenticating at: {}", url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RegistryError::AuthFailed(format!(
                "token endpoint returned {}",
                response.status()
            )));
        }

        let token_response: TokenResponse = response.json().await?;
        let token = token_response
            .get_token()
            .ok_or_else(|| RegistryError::AuthFailed("no token in response".to_string()))?
            .to_string();

        let key = format!("{}/{}", reference.registry, reference.repository);
        self.tokens.write().await.insert(key, token.clone());

        Ok(token)
    }

    async fn request_with_auth(
        &self,
        url: &str,
        reference: &ImageReference,
        accept: &[&str],
    ) -> Result<reqwest::Response, RegistryError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_str(&accept.join(", ")).unwrap());

        if let Some(token) = self.get_token(reference).await? {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            );
        }

        let response = self.client.get(url).headers(headers.clone()).send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let www_auth = response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            let token = self.authenticate(reference, www_auth).await?;

            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            );

            let response = self.client.get(url).headers(headers).send().await?;
            Ok(response)
        } else {
            Ok(response)
        }
    }

    pub async fn get_manifest(
        &self,
        reference: &ImageReference,
    ) -> Result<(Manifest, String, String), RegistryError> {
        let tag_or_digest = reference.reference();
        let url = format!(
            "{}/v2/{}/manifests/{}",
            self.registry_url(&reference.registry),
            reference.repository,
            tag_or_digest
        );

        tracing::debug!("Fetching manifest from: {}", url);

        let accept = [
            MEDIA_TYPE_MANIFEST_V2,
            MEDIA_TYPE_MANIFEST_LIST,
            MEDIA_TYPE_OCI_MANIFEST,
            MEDIA_TYPE_OCI_INDEX,
        ];

        let response = self.request_with_auth(&url, reference, &accept).await?;

        if !response.status().is_success() {
            return Err(RegistryError::ManifestNotFound(format!(
                "{}/{}:{}",
                reference.registry, reference.repository, tag_or_digest
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(MEDIA_TYPE_MANIFEST_V2)
            .to_string();

        let digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = response.text().await?;

        let manifest =
            if content_type.contains("manifest.list") || content_type.contains("image.index") {
                let list: ManifestList = serde_json::from_str(&body)?;
                Manifest::List(list)
            } else {
                let v2: ManifestV2 = serde_json::from_str(&body)?;
                Manifest::V2(v2)
            };

        Ok((manifest, content_type, digest))
    }

    pub async fn get_manifest_for_platform(
        &self,
        reference: &ImageReference,
        os: &str,
        arch: &str,
    ) -> Result<(ManifestV2, String, String), RegistryError> {
        let (manifest, content_type, digest) = self.get_manifest(reference).await?;

        match manifest {
            Manifest::V2(m) => Ok((m, content_type, digest)),
            Manifest::List(list) => {
                let platform_manifest = list
                    .manifests
                    .iter()
                    .find(|m| {
                        if let Some(p) = &m.platform {
                            p.os == os && p.architecture == arch
                        } else {
                            false
                        }
                    })
                    .ok_or_else(|| {
                        RegistryError::ManifestNotFound(format!("no manifest for {}/{}", os, arch))
                    })?;

                let mut ref_with_digest = reference.clone();
                ref_with_digest.digest = Some(platform_manifest.digest.clone());
                ref_with_digest.tag = None;

                let (manifest, content_type, digest) = self.get_manifest(&ref_with_digest).await?;

                match manifest {
                    Manifest::V2(m) => Ok((m, content_type, digest)),
                    Manifest::List(_) => Err(RegistryError::UnsupportedMediaType(
                        "nested manifest lists not supported".to_string(),
                    )),
                }
            }
        }
    }

    pub async fn get_blob(
        &self,
        reference: &ImageReference,
        digest: &str,
    ) -> Result<reqwest::Response, RegistryError> {
        let url = format!(
            "{}/v2/{}/blobs/{}",
            self.registry_url(&reference.registry),
            reference.repository,
            digest
        );

        tracing::debug!("Fetching blob: {}", digest);

        let response = self
            .request_with_auth(&url, reference, &["application/octet-stream"])
            .await?;

        if !response.status().is_success() {
            return Err(RegistryError::BlobNotFound(digest.to_string()));
        }

        Ok(response)
    }

    pub async fn get_blob_bytes(
        &self,
        reference: &ImageReference,
        digest: &str,
    ) -> Result<Vec<u8>, RegistryError> {
        let response = self.get_blob(reference, digest).await?;
        let bytes = response.bytes().await?.to_vec();
        Ok(bytes)
    }

    pub async fn get_config(
        &self,
        reference: &ImageReference,
        config_digest: &str,
    ) -> Result<ImageConfig, RegistryError> {
        let bytes = self.get_blob_bytes(reference, config_digest).await?;
        let config: ImageConfig = serde_json::from_slice(&bytes)?;
        Ok(config)
    }
}

impl Default for RegistryClient {
    fn default() -> Self {
        Self::new().expect("failed to create registry client")
    }
}

fn extract_auth_param(header: &str, param: &str) -> Option<String> {
    let search = format!("{}=\"", param);
    if let Some(start) = header.find(&search) {
        let start = start + search.len();
        if let Some(end) = header[start..].find('"') {
            return Some(header[start..start + end].to_string());
        }
    }
    None
}
