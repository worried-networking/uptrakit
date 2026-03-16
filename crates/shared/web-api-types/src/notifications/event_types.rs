use uptrakit_shared_macros::wire_safe_enum;

// ── Wire-safe Other(String) helpers ──────────────────────────────────────────
//
// These enums are serialized in API responses consumed by potentially-older
// clients. The `Other(String)` catch-all prevents deserialization failures when
// a newer server introduces a new variant. The pattern matches the canonical
// `EnrollmentStatus`/`ErrorCode` implementation in `uptrakit-wire`.

wire_safe_enum! {
    /// The type of event that triggers a notification.
    ///
    /// # Wire forward-compatibility
    ///
    /// `Other(String)` is a catch-all for event type strings received from a newer
    /// server that this client does not yet recognise. Serde deserialization is
    /// infallible: an unknown string becomes `Other(...)` rather than a parse error,
    /// allowing older clients to survive rolling upgrades without dropping the
    /// enclosing response.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
    pub enum NotificationEventType {
        UpdateAvailable => "update_available",
        UpdateCompleted => "update_completed",
        UpdateFailed => "update_failed",
        NewSoftwareDiscovered => "new_software_discovered",
        NewServiceEnrolled => "new_service_enrolled",
        CaRotated => "ca_rotated",
        BatchUpdateCompleted => "batch_update_completed",
        BatchUpdatePartiallyCompleted => "batch_update_partially_completed",
        StdinAttention => "stdin_attention",
    }
    parse_error = ParseNotificationEventTypeError("invalid notification event type");
}

wire_safe_enum! {
    /// The delivery status of a notification.
    ///
    /// # Wire forward-compatibility
    ///
    /// `Other(String)` is a catch-all for status strings received from a newer
    /// server. Serde deserialization is infallible: an unknown string becomes
    /// `Other(...)` rather than a parse error.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
    pub enum NotificationDeliveryStatus {
        Pending => "pending",
        Delivered => "delivered",
        Failed => "failed",
    }
    parse_error = ParseNotificationDeliveryStatusError("invalid notification delivery status");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All known (non-Other) `NotificationEventType` variants for iteration in tests.
    const KNOWN_EVENT_TYPES: &[NotificationEventType] = &[
        NotificationEventType::UpdateAvailable,
        NotificationEventType::UpdateCompleted,
        NotificationEventType::UpdateFailed,
        NotificationEventType::NewSoftwareDiscovered,
        NotificationEventType::NewServiceEnrolled,
        NotificationEventType::CaRotated,
        NotificationEventType::BatchUpdateCompleted,
        NotificationEventType::BatchUpdatePartiallyCompleted,
        NotificationEventType::StdinAttention,
    ];

    /// All known (non-Other) `NotificationDeliveryStatus` variants for iteration in tests.
    const KNOWN_DELIVERY_STATUSES: &[NotificationDeliveryStatus] = &[
        NotificationDeliveryStatus::Pending,
        NotificationDeliveryStatus::Delivered,
        NotificationDeliveryStatus::Failed,
    ];

    // ── NotificationEventType ───────────────────────────────────────────

    #[test]
    fn event_type_serde_round_trip() {
        for event in KNOWN_EVENT_TYPES {
            let json = serde_json::to_string(event).expect("serialization should succeed");
            let deserialized: NotificationEventType =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(&deserialized, event);
        }
    }

    #[test]
    fn event_type_other_deserializes_gracefully() {
        let json = r#""some_future_event_type""#;
        let deserialized: NotificationEventType =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(
            deserialized,
            NotificationEventType::Other("some_future_event_type".to_string())
        );
    }

    #[test]
    fn event_type_other_roundtrips() {
        let original = NotificationEventType::Other("future_event".to_string());
        let json = serde_json::to_string(&original).expect("serialization should succeed");
        assert_eq!(json, r#""future_event""#);
        let deserialized: NotificationEventType =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized, original);
    }

    #[test]
    fn event_type_as_str_values() {
        assert_eq!(
            NotificationEventType::UpdateAvailable.as_str(),
            "update_available"
        );
        assert_eq!(
            NotificationEventType::UpdateCompleted.as_str(),
            "update_completed"
        );
        assert_eq!(
            NotificationEventType::UpdateFailed.as_str(),
            "update_failed"
        );
        assert_eq!(
            NotificationEventType::NewSoftwareDiscovered.as_str(),
            "new_software_discovered"
        );
        assert_eq!(
            NotificationEventType::NewServiceEnrolled.as_str(),
            "new_service_enrolled"
        );
        assert_eq!(NotificationEventType::CaRotated.as_str(), "ca_rotated");
    }

    #[test]
    fn event_type_from_str_valid() {
        assert_eq!(
            "update_available".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateAvailable)
        );
        assert_eq!(
            "update_completed".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateCompleted)
        );
        assert_eq!(
            "update_failed".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::UpdateFailed)
        );
        assert_eq!(
            "new_software_discovered"
                .parse::<NotificationEventType>()
                .ok(),
            Some(NotificationEventType::NewSoftwareDiscovered)
        );
        assert_eq!(
            "new_service_enrolled".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::NewServiceEnrolled)
        );
        assert_eq!(
            "ca_rotated".parse::<NotificationEventType>().ok(),
            Some(NotificationEventType::CaRotated)
        );
    }

    #[test]
    fn event_type_from_str_invalid_returns_err() {
        assert!("nonexistent".parse::<NotificationEventType>().is_err());
        assert!("".parse::<NotificationEventType>().is_err());
        assert!("UPDATE_AVAILABLE".parse::<NotificationEventType>().is_err());
    }

    #[test]
    fn event_type_display_matches_as_str() {
        for event in KNOWN_EVENT_TYPES {
            assert_eq!(format!("{event}"), event.as_str());
        }
    }

    #[test]
    fn event_type_as_str_round_trips_through_from_str() {
        for event in KNOWN_EVENT_TYPES {
            let s = event.as_str();
            let parsed: NotificationEventType = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, event);
        }
    }

    #[test]
    fn parse_event_type_error_display_message() {
        let err = ParseNotificationEventTypeError;
        assert_eq!(err.to_string(), "invalid notification event type");
    }

    // ── NotificationDeliveryStatus ──────────────────────────────────────

    #[test]
    fn delivery_status_serde_round_trip() {
        for status in KNOWN_DELIVERY_STATUSES {
            let json = serde_json::to_string(status).expect("serialization should succeed");
            let deserialized: NotificationDeliveryStatus =
                serde_json::from_str(&json).expect("deserialization should succeed");
            assert_eq!(&deserialized, status);
        }
    }

    #[test]
    fn delivery_status_other_deserializes_gracefully() {
        let json = r#""processing""#;
        let deserialized: NotificationDeliveryStatus =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(
            deserialized,
            NotificationDeliveryStatus::Other("processing".to_string())
        );
    }

    #[test]
    fn delivery_status_other_roundtrips() {
        let original = NotificationDeliveryStatus::Other("queued".to_string());
        let json = serde_json::to_string(&original).expect("serialization should succeed");
        assert_eq!(json, r#""queued""#);
        let deserialized: NotificationDeliveryStatus =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized, original);
    }

    #[test]
    fn delivery_status_as_str_values() {
        assert_eq!(NotificationDeliveryStatus::Pending.as_str(), "pending");
        assert_eq!(NotificationDeliveryStatus::Delivered.as_str(), "delivered");
        assert_eq!(NotificationDeliveryStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn delivery_status_from_str_valid() {
        assert_eq!(
            "pending".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Pending)
        );
        assert_eq!(
            "delivered".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Delivered)
        );
        assert_eq!(
            "failed".parse::<NotificationDeliveryStatus>().ok(),
            Some(NotificationDeliveryStatus::Failed)
        );
    }

    #[test]
    fn delivery_status_from_str_invalid_returns_err() {
        assert!("unknown".parse::<NotificationDeliveryStatus>().is_err());
        assert!("".parse::<NotificationDeliveryStatus>().is_err());
        assert!("PENDING".parse::<NotificationDeliveryStatus>().is_err());
    }

    #[test]
    fn delivery_status_display_matches_as_str() {
        for status in KNOWN_DELIVERY_STATUSES {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    #[test]
    fn delivery_status_as_str_round_trips_through_from_str() {
        for status in KNOWN_DELIVERY_STATUSES {
            let s = status.as_str();
            let parsed: NotificationDeliveryStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, status);
        }
    }

    #[test]
    fn parse_delivery_status_error_display_message() {
        let err = ParseNotificationDeliveryStatusError;
        assert_eq!(err.to_string(), "invalid notification delivery status");
    }
}
