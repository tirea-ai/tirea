use crate::{LoadedAsset, LoadedReference, SkillError, SkillResource, SkillResourceKind};
use std::collections::HashMap;

pub(super) fn load_resource_from_maps(
    references: &HashMap<(String, String), LoadedReference>,
    assets: &HashMap<(String, String), LoadedAsset>,
    skill_id: &str,
    kind: SkillResourceKind,
    path: &str,
) -> Result<SkillResource, SkillError> {
    let key = (skill_id.to_string(), path.to_string());
    match kind {
        SkillResourceKind::Reference => references
            .get(&key)
            .cloned()
            .map(SkillResource::Reference)
            .ok_or_else(|| unsupported_resource(kind, path)),
        SkillResourceKind::Asset => assets
            .get(&key)
            .cloned()
            .map(SkillResource::Asset)
            .ok_or_else(|| unsupported_resource(kind, path)),
    }
}

fn unsupported_resource(kind: SkillResourceKind, path: &str) -> SkillError {
    match kind {
        SkillResourceKind::Reference => {
            SkillError::Unsupported(format!("reference not available: {path}"))
        }
        SkillResourceKind::Asset => {
            SkillError::Unsupported(format!("asset not available: {path}"))
        }
    }
}
