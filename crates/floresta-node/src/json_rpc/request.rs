// SPDX-License-Identifier: MIT OR Apache-2.0

//! This module defines the structure for JSON-RPC requests and provides utility functions to
//! extract parameters from the request.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents a JSON-RPC request (versions 1.0 and 2.0).
pub struct RpcRequest {
    /// The JSON-RPC version, typically "2.0".
    ///
    /// For JSON-RPC 2.0, this field is required. For earlier versions, it may be omitted.
    ///
    /// Source: <`https://json-rpc.dev/docs/reference/version-diff`>
    pub jsonrpc: Option<String>,

    /// The method to be invoked, e.g., "getblock", "sendtransaction".
    pub method: String,

    /// The parameters for the method, json value that must be an array or an object.
    pub params: Option<Value>,

    /// An optional identifier for the request, which can be used to match responses.
    pub id: Value,
}

/// Some utility functions to extract parameters from the request. These
/// methods already handle the case where the parameter is missing or has an
/// unexpected type, returning an error if so.
pub mod arg_parser {

    use serde::Deserialize;
    use serde_json::Value;

    use crate::json_rpc::res::jsonrpc_interface::JsonRpcError;

    /// Errors that originate while extracting parameters from a request.
    ///
    /// These are all client mistakes rather than node failures, and they are kept separate
    /// from the rest of [`JsonRpcError`] so a caller can tell "the request was malformed"
    /// from "the node could not answer it".
    #[derive(Debug)]
    pub enum ArgParseError {
        /// A required parameter was not supplied.
        Missing {
            /// The parameter the method expected, so the client knows what to add.
            field: String,
        },

        /// A parameter was supplied but could not be read as the expected type.
        WrongType {
            /// The parameter that was wrong.
            field: String,
            /// What the deserializer objected to.
            reason: String,
        },

        /// The `params` value was neither an array nor an object.
        ///
        /// JSON-RPC allows positional or named parameters; anything else cannot be indexed.
        BadStructure {
            /// What was actually sent, echoed back for the client.
            got: String,
        },
    }

    impl core::fmt::Display for ArgParseError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Missing { field } => write!(f, "missing parameter: {field}"),
                Self::WrongType { field, reason } => {
                    write!(f, "parameter {field} has the wrong type: {reason}")
                }
                Self::BadStructure { got } => {
                    write!(f, "params must be an array or an object, got {got}")
                }
            }
        }
    }

    impl core::error::Error for ArgParseError {
        /// The deserializer's own error is already rendered into `reason`, so there is no
        /// separate source to expose.
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            None
        }
    }

    impl From<ArgParseError> for JsonRpcError {
        /// Maps a parameter problem onto the JSON-RPC error the client receives.
        fn from(e: ArgParseError) -> Self {
            match e {
                ArgParseError::Missing { field } => Self::MissingParameter(field),
                ArgParseError::WrongType { field, reason } => {
                    Self::InvalidParameterType(format!("{field}: {reason}"))
                }
                ArgParseError::BadStructure { got } => Self::InvalidParameterStructure(got),
            }
        }
    }

    /// Extracts a parameter from the request parameters at the specified index.
    ///
    /// This function can extract any type that implements `FromStr`, such as `BlockHash` or
    /// `Txid`. It checks if the parameter exists and is a valid string representation of the type.
    /// Returns an error otherwise.
    pub fn get_at<'de, T: Deserialize<'de>>(
        params: &'de Value,
        index: usize,
        field_name: &str,
    ) -> Result<T, JsonRpcError> {
        if params.is_null() {
            return Err(ArgParseError::Missing {
                field: field_name.to_string(),
            }
            .into());
        }

        let v = match (params.is_array(), params.is_object()) {
            (true, false) => params.get(index),
            (false, true) => params.get(field_name),
            _ => {
                return Err(ArgParseError::BadStructure {
                    got: (*params).to_string(),
                }
                .into());
            }
        };

        let value = v.ok_or_else(|| ArgParseError::Missing {
            field: field_name.to_string(),
        })?;

        T::deserialize(value).map_err(|e| {
            ArgParseError::WrongType {
                field: field_name.to_string(),
                reason: e.to_string(),
            }
            .into()
        })
    }

    /// Wraps a parameter extraction result so that a missing parameter yields `Ok(None)`
    /// instead of an error. Other errors are propagated unchanged.
    pub fn try_into_optional<T>(
        result: Result<T, JsonRpcError>,
    ) -> Result<Option<T>, JsonRpcError> {
        match result {
            Ok(t) => Ok(Some(t)),
            Err(JsonRpcError::MissingParameter(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Like [`get_at`], but returns `default` when the parameter is missing instead of
    /// an error. Type mismatches are still propagated as errors.
    pub fn get_with_default<'de, T: Deserialize<'de>>(
        v: &'de Value,
        index: usize,
        field_name: &str,
        default: T,
    ) -> Result<T, JsonRpcError> {
        match get_at(v, index, field_name) {
            Ok(t) => Ok(t),
            Err(JsonRpcError::MissingParameter(_)) => Ok(default),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm,
    reason = "test code: a panic is the assertion failing, which is the intent"
)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::arg_parser::ArgParseError;
    use super::arg_parser::get_at;
    use super::arg_parser::get_with_default;
    use super::arg_parser::try_into_optional;
    use crate::json_rpc::res::jsonrpc_interface::JsonRpcError;

    /// Null params means the caller sent nothing, which must name the parameter it wanted
    /// rather than failing anonymously.
    #[test]
    fn propagates_missing_parameter_on_null_params() {
        let err = get_at::<u32>(&Value::Null, 0, "height").unwrap_err();

        match err {
            JsonRpcError::MissingParameter(field) => assert_eq!(field, "height"),
            other => panic!("expected MissingParameter, got {other:?}"),
        }
    }

    /// The module's own error type names the offending field, and maps onto the JSON-RPC
    /// error the client eventually receives.
    #[test]
    fn arg_parse_errors_map_onto_the_client_facing_error() {
        let missing = ArgParseError::Missing {
            field: "height".to_string(),
        };
        assert!(missing.to_string().contains("height"));
        assert!(matches!(
            JsonRpcError::from(missing),
            JsonRpcError::MissingParameter(f) if f == "height"
        ));

        let wrong = ArgParseError::WrongType {
            field: "verbosity".to_string(),
            reason: "expected u8".to_string(),
        };
        assert!(matches!(
            JsonRpcError::from(wrong),
            JsonRpcError::InvalidParameterType(d) if d.starts_with("verbosity")
        ));

        let bad = ArgParseError::BadStructure {
            got: "\"a string\"".to_string(),
        };
        assert!(matches!(
            JsonRpcError::from(bad),
            JsonRpcError::InvalidParameterStructure(_)
        ));
    }

    /// An index past the end of the array is also a missing parameter, named.
    #[test]
    fn propagates_missing_parameter_when_index_absent() {
        let err = get_at::<u32>(&json!([]), 0, "height").unwrap_err();

        assert!(matches!(err, JsonRpcError::MissingParameter(f) if f == "height"));
    }

    /// Params must be an array or an object; anything else is a structural error that
    /// echoes what was actually sent.
    #[test]
    fn propagates_invalid_parameter_structure() {
        let err = get_at::<u32>(&json!("a bare string"), 0, "height").unwrap_err();

        match err {
            JsonRpcError::InvalidParameterStructure(got) => {
                assert!(got.contains("a bare string"));
            }
            other => panic!("expected InvalidParameterStructure, got {other:?}"),
        }
    }

    /// A present parameter of the wrong type is distinct from a missing one, and the error
    /// names the field so the client can fix the right argument.
    #[test]
    fn propagates_invalid_parameter_type() {
        let err = get_at::<u32>(&json!(["not a number"]), 0, "height").unwrap_err();

        match err {
            JsonRpcError::InvalidParameterType(detail) => assert!(detail.starts_with("height")),
            other => panic!("expected InvalidParameterType, got {other:?}"),
        }
    }

    /// Named parameters resolve by field name rather than position.
    #[test]
    fn resolves_named_parameters_by_field() {
        let params = json!({ "height": 42 });

        assert_eq!(get_at::<u32>(&params, 0, "height").unwrap(), 42);
        assert!(matches!(
            get_at::<u32>(&params, 0, "missing").unwrap_err(),
            JsonRpcError::MissingParameter(_)
        ));
    }

    /// `try_into_optional` swallows only a missing parameter; every other failure is still
    /// propagated unchanged.
    #[test]
    fn try_into_optional_only_swallows_missing() {
        let missing = get_at::<u32>(&json!([]), 0, "height");
        assert_eq!(try_into_optional(missing).unwrap(), None);

        let wrong_type = get_at::<u32>(&json!(["nope"]), 0, "height");
        assert!(matches!(
            try_into_optional(wrong_type).unwrap_err(),
            JsonRpcError::InvalidParameterType(_)
        ));
    }

    /// `get_with_default` substitutes the default for a missing parameter, but a type
    /// mismatch is still an error rather than being silently defaulted.
    #[test]
    fn get_with_default_does_not_mask_type_errors() {
        assert_eq!(
            get_with_default(&json!([]), 0, "verbosity", 1_u32).unwrap(),
            1
        );

        let err = get_with_default(&json!(["nope"]), 0, "verbosity", 1_u32).unwrap_err();
        assert!(matches!(err, JsonRpcError::InvalidParameterType(_)));
    }
}
