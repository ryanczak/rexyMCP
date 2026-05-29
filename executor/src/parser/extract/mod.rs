// Format extractors. Each turns its slice of a model response into `Candidate`s;
// the pipeline runs the formats `detect` reported through the matching extractor.

pub mod fenced;
pub mod hermes;
pub mod loose_json;
pub mod text;
pub mod xml;
pub mod yaml;
