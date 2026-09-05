//! Version boundary for generated Loop Engine protocol types.
//!
//! Phase 2 replaces this bootstrap constant with generated Protobuf bindings.

pub const PROTOCOL_VERSION: &str = "loop-engine.v1alpha1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_namespaced() {
        assert!(PROTOCOL_VERSION.starts_with("loop-engine."));
    }
}
