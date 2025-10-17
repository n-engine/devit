pub use devit_common::patch::patch_parser::{
    FilePatch, ParsedPatch, PatchHunk, PatchLine as LineCommon,
};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub number: u32,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub is_added: bool,
    #[serde(default)]
    pub is_removed: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl From<LineCommon> for Line {
    fn from(src: LineCommon) -> Self {
        Self {
            number: src.number,
            content: src.content,
            is_added: src.is_added,
            is_removed: src.is_removed,
            ..Default::default()
        }
    }
}
