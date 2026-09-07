//! Version boundary for generated Loop Engine protocol types.
//!
//! Core DTOs are always available under [`wire::v1`]. Service clients and
//! servers are opt-in features so a process does not compile RPC surfaces it
//! is not authorized to use.

pub mod artifact;
pub mod holdout;
pub mod job;
pub mod negotiation;
pub mod runtime_validation;

pub const PROTOCOL_VERSION: &str = "loop-engine.v1alpha1";
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!("../../../fixtures/contracts/protocol/v1/schema.current.binpb");

pub mod wire {
    pub mod v1 {
        include!("generated/r#loop.v1.rs");
    }

    #[cfg(feature = "audit-service")]
    pub mod audit {
        pub mod v1 {
            include!("generated/r#loop.audit.v1.rs");
        }
    }

    #[cfg(feature = "discovery-service")]
    pub mod discovery {
        pub mod v1 {
            include!("generated/r#loop.discovery.v1.rs");
        }
    }

    #[cfg(feature = "holdout-service")]
    pub mod holdout {
        pub mod v1 {
            include!("generated/r#loop.holdout.v1.rs");
        }
    }

    #[cfg(feature = "jobs-service")]
    pub mod jobs {
        pub mod v1 {
            include!("generated/r#loop.jobs.v1.rs");
        }
    }

    #[cfg(feature = "protocol-service")]
    pub mod protocol {
        pub mod v1 {
            include!("generated/r#loop.protocol.v1.rs");
        }
    }

    #[cfg(feature = "provider-service")]
    pub mod provider {
        pub mod v1 {
            include!("generated/r#loop.provider.v1.rs");
        }
    }

    #[cfg(feature = "research-service")]
    pub mod research {
        pub mod v1 {
            include!("generated/r#loop.research.v1.rs");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_namespaced() {
        assert!(PROTOCOL_VERSION.starts_with("loop-engine."));
    }

    #[test]
    fn descriptor_set_is_committed() {
        assert!(!FILE_DESCRIPTOR_SET.is_empty());
    }
}
