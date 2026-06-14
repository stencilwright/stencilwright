//! End-to-end masking tests for `EffectivePolicy::apply`.
//!
//! Per `PUNCHLIST.md` checkpoint 1. The fixture is synthetic; do not
//! paste real Example content here.

use stencil_core::{Element, MaskConfig, MaskInner, PatternRule, Place, RedactSelector, Signature};
use stencil_mask::{MaskPolicy, ValueNameMap, derive_slot};

const FIXTURE: &str = include_str!("fixtures/example_listing.html");

fn default_patterns() -> Vec<PatternRule> {
    vec![
        PatternRule {
            name: "currency".into(),
            regex: r"\$[0-9][0-9,]*(?:\.[0-9]+)?".into(),
        },
        PatternRule {
            name: "long_digits".into(),
            regex: r"[0-9]{8,}".into(),
        },
        PatternRule {
            name: "email".into(),
            regex: r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}".into(),
        },
    ]
}

fn config_with(redact: Vec<&str>, max_unmasked_chars: usize) -> MaskConfig {
    MaskConfig {
        mask: MaskInner {
            patterns: default_patterns(),
            redact_selectors: redact
                .into_iter()
                .map(|s| RedactSelector {
                    selector: s.to_string(),
                })
                .collect(),
        },
        max_unmasked_chars,
    }
}

fn place_with_unmasks(name: &str, selectors: &[&str]) -> Place {
    Place {
        name: name.to_string(),
        url: None,
        from: None,
        via: None,
        interactive: false,
        submit: None,
        signature: Signature::default(),
        completion: None,
        redirect: None,
        elements: selectors
            .iter()
            .enumerate()
            .map(|(i, s)| Element {
                name: format!("auto_{i}"),
                selector: s.to_string(),
                auto_fill: None,
                unmasked: true,
            })
            .collect(),
    }
}

fn empty_place(name: &str) -> Place {
    place_with_unmasks(name, &[])
}

#[test]
fn default_deny_replaces_all_text_with_text_markers() {
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let vn = ValueNameMap::new();
    let out = effective.apply(FIXTURE, &vn).unwrap().0;

    // Structure preserved.
    assert!(
        out.contains("<app-shell>"),
        "missing structural tag: {out}"
    );
    assert!(out.contains("<app-post>"), "missing post element");
    assert!(out.contains("<table class=\"stats\">"));
    assert!(out.contains("data-author"));

    // Default-deny: TEXT markers present.
    assert!(out.contains("[TEXT:"), "no [TEXT:N] markers in output");

    // No raw fixture text values leak.
    for forbidden in [
        "Synthetic title alpha",
        "Synthetic title beta",
        "fake_author_alpha",
        "fake_author_beta",
        "Synthetic post body",
        "Another synthetic post body",
        "12345678",
        "$1,234.56",
        "a@b.co",
        "pretend top-level comment",
        "unrelated cell text",
    ] {
        assert!(
            !out.contains(forbidden),
            "default-deny leaked '{forbidden}' into masked output:\n{out}",
        );
    }

    // No slot markers in default-deny path — only [TEXT:N].
    assert!(
        !out.contains("[$"),
        "default-deny path should not emit slot markers:\n{out}",
    );
}

#[test]
fn unmask_passes_title_text_with_substring_slots() {
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = place_with_unmasks("listing_x", &["app-post a[slot='title']"]);
    let effective = policy.for_place(&place);
    let vn = ValueNameMap::new();
    let out = effective.apply(FIXTURE, &vn).unwrap().0;

    // Title prose passes through.
    assert!(
        out.contains("Synthetic title alpha mentions"),
        "expected unmasked title prose, got:\n{out}",
    );
    assert!(
        out.contains("Synthetic title beta also mentions"),
        "expected unmasked title prose, got:\n{out}",
    );

    // The embedded sensitive number does not pass through verbatim.
    assert!(
        !out.contains("12345678"),
        "12345678 leaked into unmasked output despite substring blacklist:\n{out}",
    );

    // The expected slot for the unnamed value appears.
    let expected_slot = derive_slot("12345678", &vn).render();
    assert!(
        out.contains(&expected_slot),
        "expected slot {expected_slot} not found in:\n{out}",
    );

    // Masked regions still default-deny.
    assert!(
        !out.contains("Synthetic post body"),
        "post body text leaked into masked output despite no unmask on it:\n{out}",
    );
    assert!(out.contains("[TEXT:"));
}

#[test]
fn slot_identity_same_value_yields_same_hash_slot() {
    // Use redact_selectors so each <td class="acct"> collapses to one
    // slot for the cell's value.
    let cfg = config_with(vec!["td.acct"], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let vn = ValueNameMap::new();
    let out = effective.apply(FIXTURE, &vn).unwrap().0;

    let expected = derive_slot("12345678", &vn).render();
    let count = out.matches(&expected).count();
    assert!(
        count >= 2,
        "expected slot {expected} to appear at least twice (same value → same slot), \
         saw {count} times in:\n{out}",
    );
    assert!(!out.contains("12345678"));
}

#[test]
fn named_slot_renders_user_given_name() {
    let cfg = config_with(vec!["td.acct"], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let mut vn = ValueNameMap::new();
    vn.insert("12345678".into(), "ira_account".into());
    let out = effective.apply(FIXTURE, &vn).unwrap().0;

    let expected = "[$ira_account 8-digit numeric]";
    let count = out.matches(expected).count();
    assert!(
        count >= 2,
        "expected named slot {expected} at least twice, saw {count} in:\n{out}",
    );
    assert!(!out.contains("12345678"));
    // No hash-form slot for 12345678 should appear when the named form
    // is in scope — the value→name lookup wins.
    let hash_form = derive_slot("12345678", &ValueNameMap::new()).render();
    assert!(
        !out.contains(&hash_form),
        "named slot should suppress the hash form, but {hash_form} appeared in:\n{out}",
    );
}

#[test]
fn length_cap_collapses_long_unmasked_text() {
    // Unmasked <label> with 250 chars must collapse to [TEXT:250]
    // when max_unmasked_chars = 200.
    let body: String = "x".repeat(250);
    let html =
        format!("<!DOCTYPE html><html><body><label class=\"big\">{body}</label></body></html>",);
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = place_with_unmasks("x", &["label.big"]);
    let effective = policy.for_place(&place);
    let vn = ValueNameMap::new();
    let out = effective.apply(&html, &vn).unwrap().0;

    assert!(out.contains("[TEXT:250]"), "expected [TEXT:250] in:\n{out}");
    assert!(
        !out.contains(&body),
        "raw 250-char body leaked through unmask cap"
    );
}

#[test]
fn whitespace_only_text_passes_through_unchanged() {
    // A text node consisting only of whitespace should not become
    // [TEXT:N] — masked output must remain layout-readable.
    let html = "<div>   \n   </div>";
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let vn = ValueNameMap::new();
    let out = effective.apply(html, &vn).unwrap().0;
    assert_eq!(out, "<div>   \n   </div>", "whitespace mangled: {out}");
}

#[test]
fn value_map_masks_matching_attribute_values() {
    let html = r#"<div username="sample_user" data-note="safe sample_user 12345678"></div>"#;
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let mut vn = ValueNameMap::new();
    vn.insert("sample_user".into(), "example_username".into());

    let out = effective.apply(html, &vn).unwrap().0;

    assert!(
        !out.contains("sample_user"),
        "value-map attribute leaked into masked output:\n{out}",
    );
    assert!(
        !out.contains("12345678"),
        "blacklisted numeric attribute leaked into masked output:\n{out}",
    );
    assert!(out.contains("$example_username"));
}

#[test]
fn current_user_attributes_are_redacted_as_whole_values() {
    let html = r#"<rs-current-user id="t2_internal_user_id" display-name="sample_user"></rs-current-user>"#;
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let mut vn = ValueNameMap::new();
    vn.insert("sample_user".into(), "example_username".into());

    let out = effective.apply(html, &vn).unwrap().0;

    assert!(!out.contains("t2_internal_user_id"));
    assert!(!out.contains("sample_user"));
    assert!(out.contains("$example_username"));
}

#[test]
fn raw_text_elements_are_collapsed_before_dom_walk() {
    let html = r#"<script>{"html":"<x-user account-id=\"12345678901\">secret</x-user>"}</script><main>ok</main>"#;
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let out = effective.apply(html, &ValueNameMap::new()).unwrap().0;

    assert!(out.contains("<script>[TEXT:"));
    assert!(!out.contains("12345678901"));
    assert!(!out.contains("secret"));
}

#[test]
fn comments_are_collapsed_before_dom_walk() {
    let html = r#"<div><!--?lit$123456789$-->ok</div>"#;
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let out = effective.apply(html, &ValueNameMap::new()).unwrap().0;

    assert!(out.contains("<!--[TEXT:"));
    assert!(!out.contains("123456789"));
}

#[test]
fn generated_ratio_attributes_are_redacted_as_whole_values() {
    let html = r#"<app-media post-upvote-ratio="100000000"></app-media>"#;
    let cfg = config_with(vec![], 200);
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = empty_place("x");
    let effective = policy.for_place(&place);
    let out = effective.apply(html, &ValueNameMap::new()).unwrap().0;

    assert!(out.contains("post-upvote-ratio=\"[$"));
    assert!(!out.contains("100000000"));
}
