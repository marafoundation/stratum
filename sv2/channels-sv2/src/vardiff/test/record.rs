//! The guard behind `VARDIFF.md`'s claim about itself.
//!
//! That document says its figures are "named inline, so this file cannot drift from the code without a test
//! failing". **When it was written, nothing enforced that.** No test read the file, so a renamed test left
//! the record pointing at nothing with the suite still green -- a claim about verification that was itself
//! unverified, which is the one defect class this whole branch is about.
//!
//! This is the missing half. It resolves every `file.rs::function` reference in the record against the test
//! tree, so a rename breaks the build rather than the reader.
//!
//! **It does not, and cannot, guard the numbers**, and that limit is worth stating plainly rather than
//! leaving to be discovered. The suite deliberately asserts inequalities with headroom -- `> 5`, `gap > 3`,
//! `> 1.2x` -- because at twenty to forty trials a point value is not a claim. So the figures the record
//! quotes are *printed* by those tests, not asserted by them, and a genuine numeric drift inside an
//! assertion's headroom would leave every test green and the record quietly stale. The record says so, and
//! says which of its content is contractual: the assertions are, the figures are illustrative of the run
//! that produced them.

/// Every `file.rs::function` reference in the record must name a test that exists.
///
/// `include_str!` rather than a path read at runtime, so the check is against the file as committed next to
/// this source rather than against whatever the working directory happens to hold.
#[test]
fn every_test_named_by_the_record_exists() {
    const RECORD: &str = include_str!("../../../VARDIFF.md");
    /// Each test file the record is allowed to name, with its source, so a reference to a file that does
    /// not exist fails here rather than resolving vacuously.
    const SOURCES: [(&str, &str); 12] = [
        (
            "actuator_granularity.rs",
            include_str!("actuator_granularity.rs"),
        ),
        ("classic.rs", include_str!("classic.rs")),
        ("cold_start.rs", include_str!("cold_start.rs")),
        ("outage.rs", include_str!("outage.rs")),
        ("quantisation.rs", include_str!("quantisation.rs")),
        ("record.rs", include_str!("record.rs")),
        ("regimes.rs", include_str!("regimes.rs")),
        ("relations.rs", include_str!("relations.rs")),
        ("sparse_parity.rs", include_str!("sparse_parity.rs")),
        ("tau_split.rs", include_str!("tau_split.rs")),
        ("wander.rs", include_str!("wander.rs")),
        ("window_cap_form.rs", include_str!("window_cap_form.rs")),
    ];

    // Hand-rolled rather than regex, because a dev-dependency for one scan is not worth it. A reference is
    // `<name>.rs::<fn>` inside backticks; scan for the separator and walk outward.
    let mut checked = 0usize;
    for (idx, _) in RECORD.match_indices(".rs::") {
        let before = &RECORD[..idx];
        let file_start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |p| p + 1);
        let stem = &RECORD[file_start..idx];
        // The record describes its own reference syntax, and that description contains the separator with
        // no file name in front of it. Prose, not a reference, so skip it -- an unknown *named* file still
        // fails below, and the vacuity guard catches a scan that has stopped matching real references.
        if stem.is_empty() {
            continue;
        }
        let file = format!("{stem}.rs");
        let after = &RECORD[idx + ".rs::".len()..];
        let fn_end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let function = &after[..fn_end];

        let source = SOURCES
            .iter()
            .find(|(name, _)| *name == file)
            .map(|(_, src)| *src)
            .unwrap_or_else(|| {
                panic!(
                    "VARDIFF.md names `{file}::{function}`, but {file} is not among the test sources this \
                     check reads. Either the reference is to a file that does not exist, or the file is \
                     new and belongs in SOURCES"
                )
            });
        assert!(
            source.contains(&format!("fn {function}(")),
            "VARDIFF.md names `{file}::{function}`, which does not exist. Renaming a test the record \
             cites has to update the record; that is what this check is for"
        );
        checked += 1;
    }

    // Guards its own vacuity. A record that had lost all its references, or a scan that had stopped
    // matching them, would otherwise pass by checking nothing.
    assert!(
        checked >= 5,
        "only {checked} references were found in VARDIFF.md, which is fewer than it is known to carry -- \
         the scan has stopped matching, or the record has stopped citing its evidence"
    );
}

/// The record must not claim a guarantee this module does not provide.
///
/// Specifically: it may say its *references* cannot drift, because the test above enforces that. It may not
/// say its *figures* cannot, because nothing does. The original wording claimed the stronger thing, and
/// this is the assertion that stops it coming back.
#[test]
fn the_record_scopes_its_own_integrity_claim() {
    const RECORD: &str = include_str!("../../../VARDIFF.md");

    assert!(
        RECORD.contains("illustrative"),
        "VARDIFF.md must mark its quoted figures as illustrative rather than contractual, because the \
         suite asserts inequalities and not the point values the record prints. Without that scope the \
         record promises more than any test here delivers"
    );
    assert!(
        !RECORD.contains("cannot drift from the code"),
        "VARDIFF.md must not claim its content cannot drift. Reference resolution is enforced by \
         `every_test_named_by_the_record_exists`; the figures are not enforced by anything, so the \
         unqualified claim is false and was false when first written"
    );
}
