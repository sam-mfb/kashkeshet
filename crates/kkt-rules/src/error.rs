use crate::StoreError;
use kkt_api_agent::{Code, RuleError};

impl From<StoreError> for RuleError {
    fn from(e: StoreError) -> RuleError {
        RuleError::new(Code::Internal, e.to_string())
    }
}
