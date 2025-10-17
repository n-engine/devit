#![cfg(feature = "test-utils")]

mod etld1_shape_s1 {
    use std::collections::HashMap;
    use mcp_tools::__test_domain_of;

    fn etld1_of_url(s: &str) -> String {
        __test_domain_of(s).unwrap_or_default()
    }

    #[test]
    fn cap_strict_two_for_co_uk_grouping() {
        let urls = vec![
            "https://news.bbc.co.uk/story",
            "https://static.bbc.co.uk/asset",
            "https://example.co.uk/a",
            "https://foo.bar.co.uk/b",
        ];
        let mut by_etld1: HashMap<String, usize> = HashMap::new();
        for u in urls {
            let etld1 = etld1_of_url(u);
            *by_etld1.entry(etld1).or_insert(0) += 1;
        }
        // Grouping must collapse subdomains under the same eTLD+1 where appropriate
        assert!(by_etld1.get("bbc.co.uk").is_some(), "missing bbc.co.uk group");
        assert!(
            by_etld1.get("example.co.uk").is_some() || by_etld1.get("bar.co.uk").is_some(),
            "expected another co.uk grouping present"
        );
    }

    #[test]
    fn fallback_sld_tld_for_simple_domains() {
        let urls = vec![
            "https://doc.rust-lang.org/guide",
            "https://play.rust-lang.org/",
        ];
        for u in urls {
            let etld1 = etld1_of_url(u);
            assert_eq!(etld1, "rust-lang.org");
        }
    }
}

