//! Standalone ingestion runner using the existing dex-format build.
extern crate self as backend_java;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JavaType {
    Void, Boolean, Byte, Short, Char, Int, Long, Float, Double,
    Class(String), Array(Box<JavaType>),
}
#[path = "../java_hierarchy.rs"]
mod suite;
use suite::type_compatibility;
