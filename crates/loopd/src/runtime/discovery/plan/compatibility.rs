use super::*;

#[test]
fn preceding_descriptors() {
    for descriptor in [CONTEXT_DESCRIPTOR, LIFECYCLE_DESCRIPTOR] {
        for controlled in [false, true] {
            let mut protocol = crate::test_support::command(1)
                .specification
                .protocol_selection
                .unwrap();
            protocol.enabled_features = FEATURES.iter().map(|name| (*name).into()).collect();
            if controlled {
                protocol
                    .enabled_features
                    .push("discovery.tool-context.v1".into());
                protocol.enabled_features.sort_unstable();
            }
            protocol.schema_descriptor_sha256 = Some(wire::Sha256Digest {
                value: descriptor.to_vec(),
            });
            protocol.selection_sha256 = Some(wire::Sha256Digest {
                value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
            });
            let original = protocol.encode_to_vec();
            validate_protocol(&protocol, controlled).unwrap();
            assert_eq!(protocol.encode_to_vec(), original);
            protocol.schema_descriptor_sha256.as_mut().unwrap().value[0] ^= 1;
            protocol.selection_sha256 = Some(wire::Sha256Digest {
                value: protocol_selection_sha256(&protocol).unwrap().to_vec(),
            });
            assert!(validate_protocol(&protocol, controlled).is_err());
        }
    }
}
