// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
wire_safe_enum! {
    #[doc = " The type of event that triggers a notification."] #[doc = ""] #[doc =
    " # Wire forward-compatibility"] #[doc = ""] #[doc =
    " `Other(String)` is a catch-all for event type strings received from a newer"] #[doc
    = " server that this client does not yet recognise. Serde deserialization is"] #[doc
    = " infallible: an unknown string becomes `Other(...)` rather than a parse error,"]
    #[doc = " allowing older clients to survive rolling upgrades without dropping the"]
    #[doc = " enclosing response."] #[derive(Clone, Debug, PartialEq, Eq, Hash)]
     pub enum
    NotificationEventType { UpdateAvailable => "update_available", UpdateCompleted =>
    "update_completed", UpdateFailed => "update_failed", NewSoftwareDiscovered =>
    "new_software_discovered", NewServiceEnrolled => "new_service_enrolled", CaRotated =>
    "ca_rotated", BatchUpdateCompleted => "batch_update_completed",
    BatchUpdatePartiallyCompleted => "batch_update_partially_completed", StdinAttention
    => "stdin_attention", } parse_error =
    ParseNotificationEventTypeError("invalid notification event type");
}
wire_safe_enum! {
    #[doc = " The delivery status of a notification."] #[doc = ""] #[doc =
    " # Wire forward-compatibility"] #[doc = ""] #[doc =
    " `Other(String)` is a catch-all for status strings received from a newer"] #[doc =
    " server. Serde deserialization is infallible: an unknown string becomes"] #[doc =
    " `Other(...)` rather than a parse error."] #[derive(Clone, Debug, PartialEq, Eq,
    Hash)]  pub enum
    NotificationDeliveryStatus { Pending => "pending", Delivered => "delivered", Failed
    => "failed", } parse_error =
    ParseNotificationDeliveryStatusError("invalid notification delivery status");
}
