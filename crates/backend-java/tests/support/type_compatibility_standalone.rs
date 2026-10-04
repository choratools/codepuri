//! Dependency-free runner for the compatibility tests while shared integration is in flight.
extern crate self as backend_java;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JavaType {
    Void,
    Boolean,
    Byte,
    Short,
    Char,
    Int,
    Long,
    Float,
    Double,
    Class(String),
    Array(Box<JavaType>),
}
#[path = "../type_compatibility.rs"]
mod suite;
