use crate::assets::protocol::{AssetLoadResult, AssetProtocol};
use std::str::from_utf8;

pub struct TextAsset(String);

impl TextAsset {
    pub fn get(&self) -> &str {
        &self.0
    }
}

pub struct TextAssetProtocol;

impl AssetProtocol for TextAssetProtocol {
    fn name(&self) -> &str {
        "txt"
    }

    fn on_load(&mut self, data: Vec<u8>) -> AssetLoadResult {
        let data = from_utf8(&data).unwrap().to_owned();
        AssetLoadResult::Data(Box::new(TextAsset(data)))
    }
}
