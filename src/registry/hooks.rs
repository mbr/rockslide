use super::storage::ManifestReference;

pub(crate) trait RegistryHooks: Send + Sync {
    fn on_manifest_uploaded(&self, manifest_reference: &ManifestReference) {
        let _ = manifest_reference;
    }
}

impl RegistryHooks for () {}
