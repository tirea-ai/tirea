pub(crate) fn generate_run_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::generate_run_id;

    #[test]
    fn test_generate_run_id_is_rfc4122_uuid_v7() {
        for _ in 0..8 {
            let run_id = generate_run_id();
            let parsed = uuid::Uuid::parse_str(&run_id)
                .unwrap_or_else(|_| panic!("run_id must be parseable UUID, got: {run_id}"));
            assert_eq!(
                parsed.get_variant(),
                uuid::Variant::RFC4122,
                "run_id must be RFC4122 UUID, got: {run_id}"
            );
            assert_eq!(
                parsed.get_version_num(),
                7,
                "run_id must be version 7 UUID, got: {run_id}"
            );
        }
    }
}
