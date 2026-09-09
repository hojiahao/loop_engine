use loop_protocol::provenance::{
    ProvenanceAssessment, ProvenanceComponent, ProvenanceError, ProvenanceSnapshot,
    assess_provenance,
};
use loop_protocol::wire::v1::{ResearchProvenanceFingerprint, Sha256Digest};

fn fingerprint(mask: &str) -> ResearchProvenanceFingerprint {
    assert_eq!(mask.len(), 6);
    let mut fields: Vec<_> = mask
        .bytes()
        .enumerate()
        .map(|(index, marker)| {
            assert!(matches!(marker, b'0' | b'1'));
            let mut value = vec![(index + 1) as u8; 32];
            if marker == b'1' {
                value[0] ^= 0xff;
            }
            Some(Sha256Digest { value })
        })
        .collect();
    ResearchProvenanceFingerprint {
        source_code_sha256: fields[0].take(),
        operator_registry_sha256: fields[1].take(),
        configuration_sha256: fields[2].take(),
        data_manifest_sha256: fields[3].take(),
        trading_calendar_sha256: fields[4].take(),
        environment_sha256: fields[5].take(),
    }
}

fn changed_names(changed: &[ProvenanceComponent]) -> String {
    if changed.is_empty() {
        "-".to_owned()
    } else {
        changed
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[test]
fn shared_freshness_vectors() {
    for line in include_str!("../../../tests/contracts/provenance_vectors.tsv")
        .lines()
        .skip(1)
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6);
        let recorded = ProvenanceSnapshot::try_from(&fingerprint(fields[1])).unwrap();
        let frozen = ProvenanceSnapshot::try_from(&fingerprint(fields[2])).unwrap();
        let current = (fields[3] != "-")
            .then(|| ProvenanceSnapshot::try_from(&fingerprint(fields[3])).unwrap());
        let assessment = assess_provenance(&recorded, &frozen, current.as_ref());
        let (status, changed) = match assessment {
            Ok(ProvenanceAssessment::Current) => ("current", vec![]),
            Ok(ProvenanceAssessment::Stale(changed)) => ("stale", changed),
            Ok(ProvenanceAssessment::Unresolved) => ("unresolved", vec![]),
            Err(ProvenanceError::RecordingMismatch(changed)) => ("recording_mismatch", changed),
            other => panic!("unexpected outcome for {}: {other:?}", fields[0]),
        };
        assert_eq!(status, fields[4], "{}", fields[0]);
        assert_eq!(changed_names(&changed), fields[5], "{}", fields[0]);
    }
}

#[test]
fn every_digest_is_required_and_fixed_width() {
    for (index, component) in ProvenanceComponent::ALL.into_iter().enumerate() {
        for size in [None, Some(0), Some(31), Some(33), Some(1_024)] {
            let mut value = fingerprint("000000");
            let field = match index {
                0 => &mut value.source_code_sha256,
                1 => &mut value.operator_registry_sha256,
                2 => &mut value.configuration_sha256,
                3 => &mut value.data_manifest_sha256,
                4 => &mut value.trading_calendar_sha256,
                _ => &mut value.environment_sha256,
            };
            *field = size.map(|size| Sha256Digest {
                value: vec![1; size],
            });
            assert_eq!(
                ProvenanceSnapshot::try_from(&value),
                Err(ProvenanceError::InvalidDigest(component))
            );
        }
    }
}

#[test]
fn snapshot_does_not_alias_wire_bytes() {
    let mut wire = fingerprint("000000");
    let original = ProvenanceSnapshot::try_from(&wire).unwrap();
    wire.source_code_sha256.as_mut().unwrap().value[0] ^= 0xff;
    assert_eq!(
        original.differences(&ProvenanceSnapshot::try_from(&wire).unwrap()),
        vec![ProvenanceComponent::SourceCode]
    );
    assert_eq!(
        original,
        ProvenanceSnapshot::try_from(&fingerprint("000000")).unwrap()
    );
}

#[test]
fn only_current_metrics_pass_the_gate() {
    let frozen = ProvenanceSnapshot::try_from(&fingerprint("000000")).unwrap();
    let changed = ProvenanceSnapshot::try_from(&fingerprint("000010")).unwrap();
    assert_eq!(
        assess_provenance(&frozen, &frozen, Some(&frozen))
            .unwrap()
            .require_current(),
        Ok(())
    );
    assert_eq!(
        assess_provenance(&frozen, &frozen, Some(&changed))
            .unwrap()
            .require_current(),
        Err(ProvenanceError::Stale(vec![
            ProvenanceComponent::TradingCalendar
        ]))
    );
    assert_eq!(
        assess_provenance(&frozen, &frozen, None)
            .unwrap()
            .require_current(),
        Err(ProvenanceError::UnresolvedCurrent)
    );
}
