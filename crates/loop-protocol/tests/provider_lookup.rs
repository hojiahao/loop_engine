#![cfg(feature = "provider-service")]

use loop_protocol::wire::provider::v1::{
    InvocationState, LookupInvocationRequest, LookupInvocationResponse,
};
use loop_protocol::wire::v1::content_block::Content;
use prost::Message;

#[test]
fn lookup_request() {
    let request = LookupInvocationRequest::decode(
        include_bytes!("../../../fixtures/contracts/protocol/v1/provider_lookup_v1.binpb")
            .as_slice(),
    )
    .expect("shared lookup request fixture");
    let context = request.context.as_ref().expect("fresh lookup context");
    assert_eq!(
        context.request_id.as_ref().expect("lookup ID").value,
        "lookup.1"
    );
    assert_eq!(
        request
            .original_request_id
            .as_ref()
            .expect("original ID")
            .value,
        "invoke.1"
    );
    assert_eq!(
        request
            .original_idempotency_key
            .as_ref()
            .expect("original key")
            .value,
        "invoke-key.1"
    );
    assert_eq!(
        request
            .request_sha256
            .as_ref()
            .expect("request digest")
            .value,
        (0_u8..32).collect::<Vec<_>>()
    );
    assert_eq!(
        LookupInvocationRequest::decode(request.encode_to_vec().as_slice()).unwrap(),
        request
    );
}

#[test]
fn lookup_result() {
    let result = LookupInvocationResponse::decode(
        include_bytes!("../../../fixtures/contracts/protocol/v1/provider_completed_v1.binpb")
            .as_slice(),
    )
    .expect("shared completed lookup fixture");
    assert_eq!(result.state(), InvocationState::Completed);
    let response = result.response.as_ref().expect("completed response");
    assert_eq!(
        response.request_id.as_ref().expect("original ID").value,
        "invoke.1"
    );
    let Some(Content::Text(text)) = response.content[0].content.as_ref() else {
        panic!("fixture must contain text content");
    };
    assert_eq!(text.text, "fixture result");
    assert!(
        response
            .usage
            .as_ref()
            .expect("usage")
            .charged_cost
            .is_none()
    );
    let reserve = result.reserved_cost.as_ref().expect("reservation");
    assert_eq!(reserve.amount.as_ref().expect("decimal").value, "0.125");
    assert_eq!(reserve.currency_code, "USD");
    assert_eq!(
        LookupInvocationResponse::decode(result.encode_to_vec().as_slice()).unwrap(),
        result
    );
}

#[test]
fn lookup_unknown() {
    let result = LookupInvocationResponse::decode([8, 127].as_slice()).unwrap();
    assert_eq!(result.state, 127);
    assert!(InvocationState::try_from(result.state).is_err());
    assert!(result.response.is_none());
    assert!(result.reserved_cost.is_none());
}
