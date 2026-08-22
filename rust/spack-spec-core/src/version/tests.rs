// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Transcription of the pure (string-based) tests from `lib/spack/spack/test/versions.py`.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use super::*;

/// `ver(a)` for a string argument.
fn v(s: &str) -> VersionUnion {
    from_string(s).unwrap()
}

/// `ver(a)` / `VersionList(a)` for a list-of-strings argument.
fn vl(items: &[&str]) -> VersionUnion {
    let mut list = VersionList::new();
    for s in items {
        list.add(v(s)).unwrap();
    }
    VersionUnion::List(list)
}

/// `Version(s)` factory.
fn pv(s: &str) -> VersionUnion {
    parse_version(s).unwrap()
}

/// `StandardVersion.from_string(s)`.
fn sv(s: &str) -> StandardVersion {
    StandardVersion::from_string(s).unwrap()
}

/// `GitVersion(s)`.
fn gv(s: &str) -> GitVersion {
    GitVersion::from_string(s).unwrap()
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Asserts the results of comparisons when 'a' is less than 'b'.
fn assert_ver_lt(a: &str, b: &str) {
    let (a, b) = (v(a), v(b));
    assert_eq!(a.compare(&b).unwrap(), Ordering::Less);
    assert_eq!(b.compare(&a).unwrap(), Ordering::Greater);
    assert_ne!(a, b);
}

/// Asserts the results of comparisons when 'a' is greater than 'b'.
fn assert_ver_gt(a: &str, b: &str) {
    assert_ver_lt(b, a);
}

/// Asserts the results of comparisons when 'a' is equal to 'b'.
fn assert_ver_eq(a: &str, b: &str) {
    let (a, b) = (v(a), v(b));
    assert_eq!(a.compare(&b).unwrap(), Ordering::Equal);
    assert_eq!(a, b);
}

/// Asserts that 'needle' is in 'haystack'.
fn assert_in(needle: &VersionUnion, haystack: &VersionUnion) {
    assert!(haystack.contains(needle).unwrap());
}

/// Asserts that 'needle' is not in 'haystack'.
fn assert_not_in(needle: &VersionUnion, haystack: &VersionUnion) {
    assert!(!haystack.contains(needle).unwrap());
}

/// Asserts that a redundant list is reduced to canonical form.
fn assert_canonical(canonical: &[&str], redundant: &[&str]) {
    assert_eq!(vl(canonical), vl(redundant));
}

/// Asserts that two version ranges overlap.
fn assert_overlaps(v1: &VersionUnion, v2: &VersionUnion) {
    assert!(v1.overlaps(v2).unwrap());
}

/// Asserts that two version ranges do not overlap.
fn assert_no_overlap(v1: &VersionUnion, v2: &VersionUnion) {
    assert!(!v1.overlaps(v2).unwrap());
}

/// Asserts that 'v1' satisfies 'v2'.
fn assert_satisfies(v1: &VersionUnion, v2: &VersionUnion) {
    assert!(v1.satisfies(v2).unwrap());
}

/// Asserts that 'v1' does not satisfy 'v2'.
fn assert_does_not_satisfy(v1: &VersionUnion, v2: &VersionUnion) {
    assert!(!v1.satisfies(v2).unwrap());
}

/// Asserts that 'a' intersect 'b' == 'expected'.
fn check_intersection(expected: &VersionUnion, a: &VersionUnion, b: &VersionUnion) {
    assert_eq!(*expected, a.intersection(b).unwrap());
}

/// Asserts that 'a' union 'b' == 'expected'.
fn check_union(expected: &VersionUnion, a: &VersionUnion, b: &VersionUnion) {
    assert_eq!(*expected, a.union(b).unwrap());
}

#[test]
fn test_string_prefix() {
    assert_ver_eq("=xsdk-0.2.0", "=xsdk-0.2.0");
    assert_ver_lt("=xsdk-0.2.0", "=xsdk-0.3");
    assert_ver_gt("=xsdk-0.3", "=xsdk-0.2.0");
}

#[test]
fn test_two_segments() {
    assert_ver_eq("=1.0", "=1.0");
    assert_ver_lt("=1.0", "=2.0");
    assert_ver_gt("=2.0", "=1.0");
}

#[test]
fn test_develop() {
    assert_ver_eq("=develop", "=develop");
    assert_ver_eq("=develop.local", "=develop.local");
    assert_ver_lt("=1.0", "=develop");
    assert_ver_gt("=develop", "=1.0");
    assert_ver_eq("=1.develop", "=1.develop");
    assert_ver_lt("=1.1", "=1.develop");
    assert_ver_gt("=1.develop", "=1.0");
    assert_ver_gt("=0.5.develop", "=0.5");
    assert_ver_lt("=0.5", "=0.5.develop");
    assert_ver_lt("=1.develop", "=2.1");
    assert_ver_gt("=2.1", "=1.develop");
    assert_ver_lt("=1.develop.1", "=1.develop.2");
    assert_ver_gt("=1.develop.2", "=1.develop.1");
    assert_ver_lt("=develop.1", "=develop.2");
    assert_ver_gt("=develop.2", "=develop.1");
    // other +infinity versions
    assert_ver_gt("=master", "=9.0");
    assert_ver_gt("=head", "=9.0");
    assert_ver_gt("=trunk", "=9.0");
    assert_ver_gt("=develop", "=9.0");
    // hierarchical develop-like versions
    assert_ver_gt("=develop", "=master");
    assert_ver_gt("=master", "=head");
    assert_ver_gt("=head", "=trunk");
    assert_ver_gt("=9.0", "=system");
    // not develop
    assert_ver_lt("=mydevelopmentnightmare", "=1.1");
    assert_ver_lt("=1.mydevelopmentnightmare", "=1.1");
    assert_ver_gt("=1.1", "=1.mydevelopmentnightmare");
}

#[test]
fn test_isdevelop() {
    assert!(sv("develop").isdevelop());
    assert!(sv("develop.1").isdevelop());
    assert!(sv("develop.local").isdevelop());
    assert!(sv("master").isdevelop());
    assert!(sv("head").isdevelop());
    assert!(sv("trunk").isdevelop());
    assert!(sv("1.develop").isdevelop());
    assert!(sv("1.develop.2").isdevelop());
    assert!(!sv("1.1").isdevelop());
    assert!(!sv("1.mydevelopmentnightmare.3").isdevelop());
    assert!(!sv("mydevelopmentnightmare.3").isdevelop());
}

#[test]
fn test_three_segments() {
    assert_ver_eq("=2.0.1", "=2.0.1");
    assert_ver_lt("=2.0", "=2.0.1");
    assert_ver_gt("=2.0.1", "=2.0");
}

#[test]
fn test_alpha() {
    assert_ver_eq("=2.0.1a", "=2.0.1a");
    assert_ver_gt("=2.0.1a", "=2.0.1");
    assert_ver_lt("=2.0.1", "=2.0.1a");
}

#[test]
fn test_patch() {
    assert_ver_eq("=5.5p1", "=5.5p1");
    assert_ver_lt("=5.5p1", "=5.5p2");
    assert_ver_gt("=5.5p2", "=5.5p1");
    assert_ver_eq("=5.5p10", "=5.5p10");
    assert_ver_lt("=5.5p1", "=5.5p10");
    assert_ver_gt("=5.5p10", "=5.5p1");
}

#[test]
fn test_num_alpha_with_no_separator() {
    assert_ver_lt("=10xyz", "=10.1xyz");
    assert_ver_gt("=10.1xyz", "=10xyz");
    assert_ver_eq("=xyz10", "=xyz10");
    assert_ver_lt("=xyz10", "=xyz10.1");
    assert_ver_gt("=xyz10.1", "=xyz10");
}

#[test]
fn test_alpha_with_dots() {
    assert_ver_eq("=xyz.4", "=xyz.4");
    assert_ver_lt("=xyz.4", "=8");
    assert_ver_gt("=8", "=xyz.4");
    assert_ver_lt("=xyz.4", "=2");
    assert_ver_gt("=2", "=xyz.4");
}

#[test]
fn test_nums_and_patch() {
    assert_ver_lt("=5.5p2", "=5.6p1");
    assert_ver_gt("=5.6p1", "=5.5p2");
    assert_ver_lt("=5.6p1", "=6.5p1");
    assert_ver_gt("=6.5p1", "=5.6p1");
}

#[test]
fn test_prereleases() {
    // pre-releases are special: they are less than final releases
    assert_ver_lt("=6.0alpha", "=6.0alpha0");
    assert_ver_lt("=6.0alpha0", "=6.0alpha1");
    assert_ver_lt("=6.0alpha1", "=6.0alpha2");
    assert_ver_lt("=6.0alpha2", "=6.0beta");
    assert_ver_lt("=6.0beta", "=6.0beta0");
    assert_ver_lt("=6.0beta0", "=6.0beta1");
    assert_ver_lt("=6.0beta1", "=6.0beta2");
    assert_ver_lt("=6.0beta2", "=6.0rc");
    assert_ver_lt("=6.0rc", "=6.0rc0");
    assert_ver_lt("=6.0rc0", "=6.0rc1");
    assert_ver_lt("=6.0rc1", "=6.0rc2");
    assert_ver_lt("=6.0rc2", "=6.0");
}

#[test]
fn test_alpha_beta() {
    // these are not pre-releases, but ordinary string components.
    assert_ver_gt("=10b2", "=10a1");
    assert_ver_lt("=10a2", "=10b2");
}

#[test]
fn test_double_alpha() {
    assert_ver_eq("=1.0aa", "=1.0aa");
    assert_ver_lt("=1.0a", "=1.0aa");
    assert_ver_gt("=1.0aa", "=1.0a");
}

#[test]
fn test_padded_numbers() {
    assert_ver_eq("=10.0001", "=10.0001");
    assert_ver_eq("=10.0001", "=10.1");
    assert_ver_eq("=10.1", "=10.0001");
    assert_ver_lt("=10.0001", "=10.0039");
    assert_ver_gt("=10.0039", "=10.0001");
}

#[test]
fn test_close_numbers() {
    assert_ver_lt("=4.999.9", "=5.0");
    assert_ver_gt("=5.0", "=4.999.9");
}

#[test]
fn test_date_stamps() {
    assert_ver_eq("=20101121", "=20101121");
    assert_ver_lt("=20101121", "=20101122");
    assert_ver_gt("=20101122", "=20101121");
}

#[test]
fn test_underscores() {
    assert_ver_eq("=2_0", "=2_0");
    assert_ver_eq("=2.0", "=2_0");
    assert_ver_eq("=2_0", "=2.0");
    assert_ver_eq("=2-0", "=2_0");
    assert_ver_eq("=2_0", "=2-0");
}

#[test]
fn test_rpm_oddities() {
    assert_ver_eq("=1b.fc17", "=1b.fc17");
    assert_ver_lt("=1b.fc17", "=1.fc17");
    assert_ver_gt("=1.fc17", "=1b.fc17");
    assert_ver_eq("=1g.fc17", "=1g.fc17");
    assert_ver_gt("=1g.fc17", "=1.fc17");
    assert_ver_lt("=1.fc17", "=1g.fc17");
}

#[test]
fn test_version_ranges() {
    assert_ver_lt("1.2:1.4", "1.6");
    assert_ver_gt("1.6", "1.2:1.4");
    assert_ver_eq("1.2:1.4", "1.2:1.4");
    assert_ne!(v("1.2:1.4"), v("1.2:1.6"));

    assert_ver_lt("1.2:1.4", "1.5:1.6");
    assert_ver_gt("1.5:1.6", "1.2:1.4");
}

#[test]
fn test_version_range_with_prereleases() {
    // 1.2.1: means from the 1.2.1 release onwards
    assert_does_not_satisfy(&v("1.2.1alpha1"), &v("1.2.1:"));
    assert_does_not_satisfy(&v("1.2.1beta2"), &v("1.2.1:"));
    assert_does_not_satisfy(&v("1.2.1rc3"), &v("1.2.1:"));

    // Pre-releases of 1.2.1 are included in the 1.2.0: range
    assert_satisfies(&v("1.2.1alpha1"), &v("1.2.0:"));
    assert_satisfies(&v("1.2.1beta1"), &v("1.2.0:"));
    assert_satisfies(&v("1.2.1rc3"), &v("1.2.0:"));

    // In Spack 1.2 and 1.2.0 are distinct with 1.2 < 1.2.0. So a lowerbound on 1.2 includes
    // pre-releases of 1.2.0 as well.
    assert_satisfies(&v("1.2.0alpha1"), &v("1.2:"));
    assert_satisfies(&v("1.2.0beta2"), &v("1.2:"));
    assert_satisfies(&v("1.2.0rc3"), &v("1.2:"));

    // An upperbound :1.1 does not include 1.2.0 pre-releases
    assert_does_not_satisfy(&v("1.2.0alpha1"), &v(":1.1"));
    assert_does_not_satisfy(&v("1.2.0beta2"), &v(":1.1"));
    assert_does_not_satisfy(&v("1.2.0rc3"), &v(":1.1"));

    assert_satisfies(&v("1.2.0alpha1"), &v(":1.2"));
    assert_satisfies(&v("1.2.0beta2"), &v(":1.2"));
    assert_satisfies(&v("1.2.0rc3"), &v(":1.2"));

    // You can also construct ranges from prereleases
    assert_satisfies(&v("1.2.0alpha2:1.2.0beta1"), &v("1.2.0alpha1:1.2.0beta2"));
    assert_satisfies(&v("1.2.0"), &v("1.2.0alpha1:"));
    assert_satisfies(&v("=1.2.0"), &v("1.2.0alpha1:"));
    assert_does_not_satisfy(&v("=1.2.0"), &v(":1.2.0rc345"));
}

#[test]
fn test_contains() {
    assert_in(&v("=1.3"), &v("1.2:1.4"));
    assert_in(&v("=1.2.5"), &v("1.2:1.4"));
    assert_in(&v("=1.3.5"), &v("1.2:1.4"));
    assert_in(&v("=1.3.5-7"), &v("1.2:1.4"));
    assert_not_in(&v("=1.1"), &v("1.2:1.4"));
    assert_not_in(&v("=1.5"), &v("1.2:1.4"));
    assert_not_in(&v("=1.5"), &v("1.5.1:1.6"));
    assert_not_in(&v("=1.5"), &v("1.5.1:"));

    assert_in(&v("=1.4.2"), &v("1.2:1.4"));
    assert_not_in(&v("=1.4.2"), &v("1.2:1.4.0"));

    assert_in(&v("=1.2.8"), &v("1.2.7:1.4"));
    assert_in(&v("1.2.7:1.4"), &v(":"));
    assert_not_in(&v("=1.2.5"), &v("1.2.7:1.4"));

    assert_in(&v("=1.4.1"), &v("1.2.7:1.4"));
    assert_not_in(&v("=1.4.1"), &v("1.2.7:1.4.0"));
}

#[test]
fn test_in_list() {
    assert_in(&v("1.2"), &vl(&["1.5", "1.2", "1.3"]));
    assert_in(&v("1.2.5"), &vl(&["1.5", "1.2:1.3"]));
    assert_in(&v("1.5"), &vl(&["1.5", "1.2:1.3"]));
    assert_not_in(&v("1.4"), &vl(&["1.5", "1.2:1.3"]));

    assert_in(&v("1.2.5:1.2.7"), &vl(&[":"]));
    assert_in(&v("1.2.5:1.2.7"), &vl(&["1.5", "1.2:1.3"]));
    assert_not_in(&v("1.2.5:1.5"), &vl(&["1.5", "1.2:1.3"]));
    assert_not_in(&v("1.1:1.2.5"), &vl(&["1.5", "1.2:1.3"]));
}

#[test]
fn test_ranges_overlap() {
    assert_overlaps(&v("1.2"), &v("1.2"));
    assert_overlaps(&v("1.2.1"), &v("1.2.1"));
    assert_overlaps(&v("1.2.1b"), &v("1.2.1b"));

    assert_overlaps(&v("1.2:1.7"), &v("1.6:1.9"));
    assert_overlaps(&v(":1.7"), &v("1.6:1.9"));
    assert_overlaps(&v(":1.7"), &v(":1.9"));
    assert_overlaps(&v(":1.7"), &v("1.6:"));
    assert_overlaps(&v("1.2:"), &v("1.6:1.9"));
    assert_overlaps(&v("1.2:"), &v(":1.9"));
    assert_overlaps(&v("1.2:"), &v("1.6:"));
    assert_overlaps(&v(":"), &v(":"));
    assert_overlaps(&v(":"), &v("1.6:1.9"));
    assert_overlaps(&v("1.6:1.9"), &v(":"));
}

#[test]
fn test_overlap_with_containment() {
    assert_in(&v("1.6.5"), &v("1.6"));
    assert_in(&v("1.6.5"), &v(":1.6"));

    assert_overlaps(&v("1.6.5"), &v(":1.6"));
    assert_overlaps(&v(":1.6"), &v("1.6.5"));

    assert_not_in(&v(":1.6"), &v("1.6.5"));
    assert_in(&v("1.6.5"), &v(":1.6"));
}

#[test]
fn test_lists_overlap() {
    assert_overlaps(&v("1.2b:1.7,5"), &v("1.6:1.9,1"));
    assert_overlaps(&v("1,2,3,4,5"), &v("3,4,5,6,7"));
    assert_overlaps(&v("1,2,3,4,5"), &v("5,6,7"));
    assert_overlaps(&v("1,2,3,4,5"), &v("5:7"));
    assert_overlaps(&v("1,2,3,4,5"), &v("3, 6:7"));
    assert_overlaps(&v("1, 2, 4, 6.5"), &v("3, 6:7"));
    assert_overlaps(&v("1, 2, 4, 6.5"), &v(":, 5, 8"));
    assert_overlaps(&v("1, 2, 4, 6.5"), &v(":"));
    assert_no_overlap(&v("1, 2, 4"), &v("3, 6:7"));
    assert_no_overlap(&v("1,2,3,4,5"), &v("6,7"));
    assert_no_overlap(&v("1,2,3,4,5"), &v("6:7"));
}

#[test]
fn test_canonicalize_list() {
    assert_canonical(&["1.2", "1.3", "1.4"], &["1.2", "1.3", "1.3", "1.4"]);
    assert_canonical(&["1.2", "1.3:1.4"], &["1.2", "1.3", "1.3:1.4"]);
    assert_canonical(&["1.2", "1.3:1.4"], &["1.2", "1.3:1.4", "1.4"]);
    assert_canonical(&["1.3:1.4"], &["1.3:1.4", "1.3", "1.3.1", "1.3.9", "1.4"]);
    assert_canonical(&["1.3:1.4"], &["1.3", "1.3.1", "1.3.9", "1.4", "1.3:1.4"]);
    assert_canonical(
        &["1.3:1.5"],
        &["1.3", "1.3.1", "1.3.9", "1.4:1.5", "1.3:1.4"],
    );
    assert_canonical(&["1.3:1.5"], &["1.3, 1.3.1,1.3.9,1.4:1.5,1.3:1.4"]);
    assert_canonical(&["1.3:1.5"], &["1.3, 1.3.1,1.3.9,1.4 : 1.5 , 1.3 : 1.4"]);
    assert_canonical(&[":"], &[":,1.3, 1.3.1,1.3.9,1.4 : 1.5 , 1.3 : 1.4"]);
}

#[test]
fn test_intersection() {
    check_intersection(&v("2.5"), &v("1.0:2.5"), &v("2.5:3.0"));
    check_intersection(&v("2.5:2.7"), &v("1.0:2.7"), &v("2.5:3.0"));
    check_intersection(&v("0:1"), &v(":"), &v("0:1"));

    check_intersection(
        &vl(&["1.0", "2.5:2.7"]),
        &vl(&["1.0:2.7"]),
        &vl(&["2.5:3.0", "1.0"]),
    );
    check_intersection(
        &vl(&["2.5:2.7"]),
        &vl(&["1.1:2.7"]),
        &vl(&["2.5:3.0", "1.0"]),
    );
    check_intersection(&vl(&["0:1"]), &vl(&[":"]), &vl(&["0:1"]));

    check_intersection(
        &vl(&["=ref=1.0", "=1.1"]),
        &vl(&["=ref=1.0", "1.1"]),
        &vl(&["1:1.0", "=1.1"]),
    );
}

#[test]
fn test_intersect_with_containment() {
    check_intersection(&v("1.6.5"), &v("1.6.5"), &v(":1.6"));
    check_intersection(&v("1.6.5"), &v(":1.6"), &v("1.6.5"));

    check_intersection(&v("1.6:1.6.5"), &v(":1.6.5"), &v("1.6"));
    check_intersection(&v("1.6:1.6.5"), &v("1.6"), &v(":1.6.5"));

    check_intersection(&v("11.2"), &v("11"), &v("11.2"));
    check_intersection(&v("11.2"), &v("11.2"), &v("11"));
}

#[test]
fn test_union_with_containment() {
    check_union(&v(":1.6"), &v("1.6.5"), &v(":1.6"));
    check_union(&v(":1.6"), &v(":1.6"), &v("1.6.5"));

    check_union(&v(":1.6"), &v(":1.6.5"), &v("1.6"));
    check_union(&v(":1.6"), &v("1.6"), &v(":1.6.5"));

    check_union(&v(":"), &v("1.0:"), &v(":2.0"));

    check_union(&v("1:4"), &v("1:3"), &v("2:4"));
    check_union(&v("1:4"), &v("2:4"), &v("1:3"));

    // Tests successor/predecessor case.
    check_union(&v("1:4"), &v("1:2"), &v("3:4"));

    check_union(
        &vl(&["1:1.0", "1.1"]),
        &vl(&["=ref=1.0", "1.1"]),
        &vl(&["1:1.0", "=1.1"]),
    );
}

#[test]
fn test_basic_version_satisfaction() {
    assert_satisfies(&v("4.7.3"), &v("4.7.3"));

    assert_satisfies(&v("4.7.3"), &v("4.7"));
    assert_satisfies(&v("4.7.3v2"), &v("4.7"));
    assert_satisfies(&v("4.7v6"), &v("4.7"));

    assert_satisfies(&v("4.7.3"), &v("4"));
    assert_satisfies(&v("4.7.3v2"), &v("4"));
    assert_satisfies(&v("4.7v6"), &v("4"));

    assert_does_not_satisfy(&v("4.8.0"), &v("4.9"));
    assert_does_not_satisfy(&v("4.8"), &v("4.9"));
    assert_does_not_satisfy(&v("4"), &v("4.9"));
}

#[test]
fn test_basic_version_satisfaction_in_lists() {
    assert_satisfies(&vl(&["4.7.3"]), &vl(&["4.7.3"]));

    assert_satisfies(&vl(&["4.7.3"]), &vl(&["4.7"]));
    assert_satisfies(&vl(&["4.7.3v2"]), &vl(&["4.7"]));
    assert_satisfies(&vl(&["4.7v6"]), &vl(&["4.7"]));

    assert_satisfies(&vl(&["4.7.3"]), &vl(&["4"]));
    assert_satisfies(&vl(&["4.7.3v2"]), &vl(&["4"]));
    assert_satisfies(&vl(&["4.7v6"]), &vl(&["4"]));

    assert_does_not_satisfy(&vl(&["4.8.0"]), &vl(&["4.9"]));
    assert_does_not_satisfy(&vl(&["4.8"]), &vl(&["4.9"]));
    assert_does_not_satisfy(&vl(&["4"]), &vl(&["4.9"]));
}

#[test]
fn test_version_range_satisfaction() {
    assert_satisfies(&v("4.7b6"), &v("4.3:4.7"));
    assert_satisfies(&v("4.3.0"), &v("4.3:4.7"));
    assert_satisfies(&v("4.3.2"), &v("4.3:4.7"));

    assert_does_not_satisfy(&v("4.8.0"), &v("4.3:4.7"));
    assert_does_not_satisfy(&v("4.3"), &v("4.4:4.7"));

    assert_satisfies(&v("4.7b6"), &v("4.3:4.7"));
    assert_does_not_satisfy(&v("4.8.0"), &v("4.3:4.7"));
}

#[test]
fn test_version_range_satisfaction_in_lists() {
    assert_satisfies(&vl(&["4.7b6"]), &vl(&["4.3:4.7"]));
    assert_satisfies(&vl(&["4.3.0"]), &vl(&["4.3:4.7"]));
    assert_satisfies(&vl(&["4.3.2"]), &vl(&["4.3:4.7"]));

    assert_does_not_satisfy(&vl(&["4.8.0"]), &vl(&["4.3:4.7"]));
    assert_does_not_satisfy(&vl(&["4.3"]), &vl(&["4.4:4.7"]));
}

#[test]
fn test_satisfaction_with_lists() {
    assert_satisfies(&v("4.7"), &v("4.3, 4.6, 4.7"));
    assert_satisfies(&v("4.7.3"), &v("4.3, 4.6, 4.7"));
    assert_satisfies(&v("4.6.5"), &v("4.3, 4.6, 4.7"));
    assert_satisfies(&v("4.6.5.2"), &v("4.3, 4.6, 4.7"));

    assert_does_not_satisfy(&v("4"), &v("4.3, 4.6, 4.7"));
    assert_does_not_satisfy(&v("4.8.0"), &v("4.2, 4.3:4.7"));

    assert_satisfies(&v("4.8.0"), &v("4.2, 4.3:4.8"));
    assert_satisfies(&v("4.8.2"), &v("4.2, 4.3:4.8"));
}

#[test]
fn test_formatted_strings() {
    let versions = [
        "1.2.3b", "1_2_3b", "1-2-3b", "1.2-3b", "1.2_3b", "1-2.3b", "1-2_3b", "1_2.3b", "1_2-3b",
    ];
    for item in versions {
        let v = sv(item);
        assert_eq!(v.dotted().string(), "1.2.3b");
        assert_eq!(v.dashed().string(), "1-2-3b");
        assert_eq!(v.underscored().string(), "1_2_3b");
        assert_eq!(v.joined().string(), "123b");

        assert_eq!(v.dotted().dashed().string(), "1-2-3b");
        assert_eq!(v.dotted().underscored().string(), "1_2_3b");
        assert_eq!(v.dotted().dotted().string(), "1.2.3b");
        assert_eq!(v.dotted().joined().string(), "123b");
    }
}

#[test]
fn test_dotted_numeric_string() {
    assert_eq!(sv("1a2b3").dotted_numeric_string(), "1.0.2.0.3");
    assert_eq!(sv("1a2b3alpha4").dotted_numeric_string(), "1.0.2.0.3.0.4");
}

#[test]
fn test_up_to() {
    let v = sv("1.23-4_5b");

    assert_eq!(v.up_to(1).string(), "1");
    assert_eq!(v.up_to(2).string(), "1.23");
    assert_eq!(v.up_to(3).string(), "1.23-4");
    assert_eq!(v.up_to(4).string(), "1.23-4_5");
    assert_eq!(v.up_to(5).string(), "1.23-4_5b");

    assert_eq!(v.up_to(-1).string(), "1.23-4_5");
    assert_eq!(v.up_to(-2).string(), "1.23-4");
    assert_eq!(v.up_to(-3).string(), "1.23");
    assert_eq!(v.up_to(-4).string(), "1");

    assert_eq!(v.up_to(2).dotted().string(), "1.23");
    assert_eq!(v.up_to(2).dashed().string(), "1-23");
    assert_eq!(v.up_to(2).underscored().string(), "1_23");
    assert_eq!(v.up_to(2).joined().string(), "123");

    assert_eq!(v.dotted().up_to(2).string(), "1.23");
    assert_eq!(v.dashed().up_to(2).string(), "1-23");
    assert_eq!(v.underscored().up_to(2).string(), "1_23");

    assert_eq!(v.up_to(2).up_to(1).string(), "1");
}

#[test]
fn test_repr_and_str() {
    for s in ["1.2.3", "R2016a", "R2016a.2-3_4"] {
        let a = pv(s);
        assert_eq!(a.to_string(), s);
        let b = pv(&a.to_string());
        assert_eq!(a, b);
        assert_eq!(a.to_string(), b.to_string());
    }
}

#[test]
fn test_str_and_hash_version_range() {
    // Precomputed string and hash values must be consistent with computed ones. The parsed
    // range caches its string; an equal, component-built range stringifies lazily.
    let parsed = match v("1.2:3.4") {
        VersionUnion::Range(r) => r,
        _ => panic!("expected a range"),
    };
    let built = ClosedOpenRange::new(sv("1.2"), next_version(&sv("3.4"))).unwrap();
    assert_eq!(parsed, built);
    assert_eq!(parsed.to_string(), "1.2:3.4");
    assert_eq!(built.to_string(), "1.2:3.4");
    assert_eq!(hash_of(&parsed), hash_of(&built));
}

#[test]
fn test_from_string_rejects_what_python_rejects() {
    // Strings `ver()` rejects in Python must be rejected here as well.
    for s in ["1:2:3", "a=b=c", "!", "1!2"] {
        assert!(matches!(from_string(s), Err(VersionError::Parse(_))), "{s}");
    }
    for s in ["18:1_7", "20.17_Z:2_3", "8-3_fc17-5:Z"] {
        assert!(
            matches!(from_string(s), Err(VersionError::EmptyRange(_))),
            "{s}"
        );
    }
    // ... and quirks Python accepts stay accepted: spaces are stripped, and `=1.2=3` parses
    // as the git ref "1.2" pinned at version 3.
    assert_eq!(v("foo 1.2"), v("foo1.2"));
    match v("=1.2=3") {
        VersionUnion::Git(g) => {
            assert_eq!(g.git_ref(), "1.2");
            assert_eq!(g.ref_version().unwrap(), &sv("3"));
        }
        other => panic!("expected GitVersion, got {other:?}"),
    }
}

#[test]
fn test_stringify_version() {
    for version_str in ["1.2string3", "1.2-3xyz_4-alpha.5", "1.2beta", "1_x_rc-4"] {
        let mut v = sv(version_str);
        v.clear_string();
        assert_eq!(v.to_string(), version_str);
        assert_eq!(v.string(), version_str);
    }
}

#[test]
fn test_len() {
    let a = sv("1.2.3.4");
    assert_eq!(a.len(), a.release().len());
    assert_eq!(a.len(), 4);
    let b = sv("2018.0");
    assert_eq!(b.len(), 2);
}

#[test]
fn test_get_item() {
    let a = sv("0.1_2-3");
    assert_eq!(a.component(1), Some(&Component::Int(1)));

    // Test slicing
    let b = a.slice(Some(0), Some(2));
    assert_eq!(b, sv("0.1"));
    assert_eq!(b.to_string(), "0.1");

    let b = a.slice(Some(0), Some(3));
    assert_eq!(b, sv("0.1_2"));
    assert_eq!(b.to_string(), "0.1_2");

    let b = a.slice(Some(1), None);
    assert_eq!(b, sv("1_2-3"));
    assert_eq!(b.to_string(), "1_2-3");

    // Out-of-bounds and negative indexing
    assert_eq!(a.component(-1), Some(&Component::Int(3)));
    assert_eq!(a.component(4), None);
}

#[test]
fn test_list_highest() {
    let vl = match vl(&["=master", "=1.2.3", "=develop", "=3.4.5", "=foobar"]) {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    assert_eq!(vl.highest(), Some(&sv("develop")));
    assert_eq!(vl.lowest(), Some(&sv("foobar")));
    assert_eq!(vl.highest_numeric(), Some(&sv("3.4.5")));

    let vl2 = match super::from_string("=master,=develop").unwrap() {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    assert_eq!(vl2.highest_numeric(), None);
    assert_eq!(vl2.preferred(), Some(&sv("develop")));
    assert_eq!(vl2.lowest(), Some(&sv("master")));
}

#[test]
fn test_invalid_versions() {
    for version_str in ["foo 1.2.0", "!", "1!2", "=1.2.0"] {
        assert!(
            matches!(parse_version(version_str), Err(VersionError::Parse(_))),
            "expected parse error for {version_str:?}"
        );
    }
}

#[test]
fn test_version_git_vs_base() {
    let cases = [
        ("1.2.9", false),
        ("gitmain", false),
        ("git.foo", true),
        ("git.abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd", true),
        ("abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd", true),
    ];
    for (string, git) in cases {
        assert_eq!(is_git_version(string), git, "{string}");
        assert_eq!(matches!(pv(string), VersionUnion::Git(_)), git, "{string}");
    }
}

#[test]
fn test_version_range_nonempty() {
    let range = VersionUnion::Range(version_range("1.2.0", "1.2").unwrap());
    assert_in(&pv("1.2.9"), &range);
    assert_in(&pv("1.1.1"), &v("1.0:1"));
}

#[test]
fn test_empty_version_range_raises() {
    let expected = "2:1.0 is an empty range";
    match version_range("2", "1.0") {
        Err(VersionError::EmptyRange(msg)) => assert_eq!(msg, expected),
        other => panic!("expected EmptyRange, got {other:?}"),
    }
    match ver("2:1.0") {
        Err(VersionError::EmptyRange(msg)) => assert_eq!(msg, expected),
        other => panic!("expected EmptyRange, got {other:?}"),
    }
}

#[test]
fn test_version_empty_slice() {
    // An empty slice yields the "empty" version instead of an error (#25953).
    assert_eq!(sv("1.").slice(Some(1), None), sv(""));
}

#[test]
fn test_version_range_satisfies_means_nonempty_intersection() {
    let x = VersionUnion::Range(version_range("3.7.0", "3").unwrap());
    let y = VersionUnion::Range(version_range("3.6.0", "3.6.0").unwrap());
    assert!(!x.satisfies(&y).unwrap());
    assert!(!y.satisfies(&x).unwrap());
}

#[test]
fn test_version_list_with_range_and_concrete_version_is_not_concrete() {
    let mut l = VersionList::new();
    l.add(pv("3.1")).unwrap();
    l.add(VersionUnion::Range(
        version_range("3.1.1", "3.1.2").unwrap(),
    ))
    .unwrap();
    assert_eq!(l.len(), 2);
    assert!(l.concrete().is_none());
}

#[test]
fn test_git_versions_store_ref_requests() {
    let sha = "a".repeat(40);
    let cases: [(&str, Option<&str>); 4] = [
        ("foo", Some("develop")),
        (&sha, Some("develop")),
        (&sha, None),
        ("v1.2.0", Some("1.2.0")),
    ];
    for (git_ref, std_version) in cases {
        let vstring = match std_version {
            Some(s) => format!("git.{git_ref}={s}"),
            None => git_ref.to_string(),
        };
        let g = match pv(&vstring) {
            VersionUnion::Git(g) => g,
            other => panic!("expected GitVersion, got {other:?}"),
        };
        assert_eq!(g.git_ref(), git_ref);
        if let Some(s) = std_version {
            assert_eq!(g.std_version(), Some(&sv(s)));
        }
        if g.is_commit() {
            assert_eq!(g.commit_sha(), Some(g.git_ref()));
        }
    }
}

#[test]
fn test_git_ref_can_be_assigned_a_version() {
    let cases = [
        (format!("{}=develop", "abc12".repeat(8)), "develop", true),
        (format!("git.{}=main", "abc12".repeat(8)), "main", true),
        (format!("{}=develop", "a".repeat(40)), "develop", true),
        (format!("{}=3.2", "b".repeat(40)), "3.2", true),
        ("git.foo=3.2".to_string(), "3.2", false),
    ];
    for (vstring, eq_vstring, is_commit) in cases {
        let g = match pv(&vstring) {
            VersionUnion::Git(g) => g,
            other => panic!("expected GitVersion, got {other:?}"),
        };
        assert_eq!(g.is_commit(), is_commit);
        assert!(!g.needs_lookup());
        assert_eq!(g.ref_version().unwrap(), &sv(eq_vstring));
    }
}

#[test]
fn test_version_intersects_satisfies_semantic() {
    let a40 = "a".repeat(40);
    let b40 = "b".repeat(40);
    let cases: [(String, String, (bool, bool, bool)); 7] = [
        // StandardVersion
        ("4.7.3".into(), "4.7.3".into(), (true, true, true)),
        ("4.7.3".into(), "4.7".into(), (true, true, false)),
        ("4.7.3".into(), "4".into(), (true, true, false)),
        ("4.7.3".into(), "4.8".into(), (false, false, false)),
        // GitVersion
        (
            format!("git.{a40}=develop"),
            "develop".into(),
            (true, true, false),
        ),
        (
            format!("git.{a40}=develop"),
            format!("git.{a40}=develop"),
            (true, true, true),
        ),
        (
            format!("git.{a40}=develop"),
            format!("git.{b40}=develop"),
            (false, false, false),
        ),
    ];
    for (lhs_str, rhs_str, (intersect, lhs_sat_rhs, rhs_sat_lhs)) in cases {
        let (lhs, rhs) = (v(&lhs_str), v(&rhs_str));
        assert_eq!(
            lhs.intersects(&rhs).unwrap(),
            intersect,
            "{lhs_str} ~ {rhs_str}"
        );
        assert_eq!(
            lhs.intersects(&rhs).unwrap(),
            rhs.intersects(&lhs).unwrap(),
            "{lhs_str} ~ {rhs_str}"
        );
        assert_eq!(
            lhs.satisfies(&rhs).unwrap(),
            lhs_sat_rhs,
            "{lhs_str} -> {rhs_str}"
        );
        assert_eq!(
            rhs.satisfies(&lhs).unwrap(),
            rhs_sat_lhs,
            "{rhs_str} -> {lhs_str}"
        );
    }
}

#[test]
fn test_total_order_versions_and_ranges() {
    // StandardVersion / GitVersion (at equal ref version)
    assert_ver_lt("=1.2", "git.ref=1.2");
    assert_ver_gt("git.ref=1.2", "=1.2");

    // StandardVersion / GitVersion (at different ref versions)
    assert_ver_lt("git.ref=1.2", "=1.3");
    assert_ver_gt("=1.3", "git.ref=1.2");
    assert_ver_lt("=1.2", "git.ref=1.3");
    assert_ver_gt("git.ref=1.3", "=1.2");

    // GitVersion / ClosedOpenRange (at equal ref/lo version)
    assert_ver_lt("git.ref=1.2", "1.2");
    assert_ver_gt("1.2", "git.ref=1.2");

    // GitVersion / ClosedOpenRange (at different ref/lo version)
    assert_ver_lt("git.ref=1.2", "1.3");
    assert_ver_gt("1.3", "git.ref=1.2");
    assert_ver_lt("1.2", "git.ref=1.3");
    assert_ver_gt("git.ref=1.3", "1.2");

    // StandardVersion / ClosedOpenRange (at equal lo version)
    assert_ver_lt("=1.2", "1.2");
    assert_ver_gt("1.2", "=1.2");

    // StandardVersion / ClosedOpenRange (at different lo version)
    assert_ver_lt("=1.2", "1.3");
    assert_ver_gt("1.3", "=1.2");
    assert_ver_lt("1.2", "=1.3");
    assert_ver_gt("=1.3", "1.2");
}

#[test]
fn test_git_version_accessors() {
    let g = gv("my_branch=1.2-3");
    let components: Vec<&Component> = g.ref_version().unwrap().release().iter().collect();
    assert_eq!(
        components,
        [&Component::Int(1), &Component::Int(2), &Component::Int(3)]
    );
    assert_eq!(g.component(0).unwrap(), Some(Component::Int(1)));
    assert_eq!(g.component(1).unwrap(), Some(Component::Int(2)));
    assert_eq!(g.component(2).unwrap(), Some(Component::Int(3)));
    assert_eq!(g.slice(Some(0), Some(2)).unwrap(), sv("1.2"));
    assert_eq!(g.slice(Some(0), Some(10)).unwrap(), sv("1.2.3"));
    assert_eq!(g.dotted().unwrap().to_string(), "1.2.3");
    assert_eq!(g.dashed().unwrap().to_string(), "1-2-3");
    assert_eq!(g.underscored().unwrap().to_string(), "1_2_3");
    assert_eq!(g.up_to(1).unwrap(), sv("1"));
    assert_eq!(g.up_to(2).unwrap(), sv("1.2"));
    assert_eq!(g.len().unwrap(), 3);
    assert!(!g.isdevelop().unwrap());
    assert!(gv("my_branch=develop").isdevelop().unwrap());
}

#[test]
fn test_version_list_normalization() {
    // Git versions and ordinary versions can live together in a VersionList
    match vl(&["=1.2", "ref=1.2"]) {
        VersionUnion::List(l) => assert_eq!(l.len(), 2),
        _ => panic!("expected a list"),
    }

    // But when a range is added, the only disjoint bit is the range.
    assert_eq!(
        vl(&["=1.2", "ref=1.2", "ref=1.3", "1.2:1.3"]),
        vl(&["1.2:1.3"])
    );

    // Also test normalization when using ver.
    assert_eq!(v("=1.0,ref=1.0,1.0:2.0"), vl(&["1.0:2.0"]));
    assert_eq!(v("=1.0,1.0:2.0,ref=1.0"), vl(&["1.0:2.0"]));
    assert_eq!(v("1.0:2.0,=1.0,ref=1.0"), vl(&["1.0:2.0"]));
}

#[test]
fn test_version_list_connected_union_of_disjoint_ranges() {
    // Lists of ranges simplify when their intersection is empty but their union is connected.
    assert_eq!(v("1.0:2.0,2.1,2.2:3,4:6"), vl(&["1.0:6"]));
    assert_eq!(v("1.0:1.2,1.3:2"), v("1.0:1.5,1.6:2"));
}

#[test]
fn test_version_comparison_with_list_fails() {
    let vlist = vl(&["=1.3"]);
    for version in ["=1.2", "git.ref=1.2", "1.2"] {
        let version = v(version);
        assert!(matches!(
            version.compare(&vlist),
            Err(VersionError::Type(_))
        ));
        assert!(matches!(
            vlist.compare(&version),
            Err(VersionError::Type(_))
        ));
    }
}

#[test]
fn test_inclusion_upperbound() {
    // Pure-version part of the spec test: @=1.2 satisfies @:1.2.0, but the range @1.2 does
    // not, since it includes e.g. 1.2.1. They do intersect, of course.
    let is_specific = v("=1.2");
    let is_range = v("1.2");
    let upperbound = v(":1.2.0");

    assert_satisfies(&is_specific, &upperbound);
    assert_does_not_satisfy(&is_range, &upperbound);
    assert!(is_specific.intersects(&upperbound).unwrap());
    assert!(is_range.intersects(&upperbound).unwrap());
}

#[test]
fn test_unresolvable_git_version_lookup_needed() {
    // A git version without a pinned version needs an external lookup: dereferencing the ref
    // version or comparing must surface LookupNeeded, not panic.
    let sha = "a".repeat(40);
    let g = gv(&sha);
    assert!(g.needs_lookup());
    match g.ref_version() {
        Err(VersionError::LookupNeeded(msg)) => {
            assert_eq!(
                msg,
                format!("git ref '{sha}' cannot be looked up: call attach_lookup first")
            )
        }
        other => panic!("expected LookupNeeded, got {other:?}"),
    }
    let unresolved = VersionUnion::Git(g);
    assert!(matches!(
        unresolved.satisfies(&v("1.0:")),
        Err(VersionError::LookupNeeded(_))
    ));
    // Stringification must not require a lookup: the =<version> suffix is just omitted.
    assert_eq!(unresolved.to_string(), sha);
}

#[test]
fn test_git_branch_with_slash() {
    // MockLookup for "git.feature/bar" returning version 1.2 at distance 0.
    let mut g = match v("git.feature/bar") {
        VersionUnion::Git(g) => g,
        other => panic!("expected GitVersion, got {other:?}"),
    };
    assert_eq!(g.git_ref(), "feature/bar");
    assert!(g.needs_lookup());
    g.resolve_lookup(Some("1.2"), 0).unwrap();
    assert_eq!(g.ref_version().unwrap(), &sv("1.2"));
    assert!(VersionUnion::Git(g).satisfies(&v("1.2")).unwrap());
}

#[test]
fn test_resolved_git_version_is_shown_in_str() {
    // A GitVersion resolved from a commit at distance 1 past version 1.0 prints as
    // <hash>=1.0-git.1, not just <hash>.
    let sha = "c".repeat(40);
    let mut g = gv(&sha);
    g.resolve_lookup(Some("1.0"), 1).unwrap();
    assert_eq!(g.to_string(), format!("{sha}=1.0-git.1"));
    // An unknown version resolves to "0".
    let mut g = gv(&sha);
    g.resolve_lookup(None, 3).unwrap();
    assert_eq!(g.ref_version().unwrap(), &sv("0-git.3"));
}

#[test]
fn test_git_version_str() {
    assert_eq!(v("git.foo=1.2").to_string(), "git.foo=1.2");
    assert_eq!(v("foo=1.2").to_string(), "foo=1.2");
    assert_eq!(v("git.foo").to_string(), "git.foo");
}

#[test]
fn test_version_list_str() {
    assert_eq!(vl(&[]).to_string(), "");
    assert_eq!(vl(&["=1.2", "ref=1.2"]).to_string(), "=1.2,ref=1.2");
    assert_eq!(vl(&["1.2:1.4", "=1.6"]).to_string(), "1.2:1.4,=1.6");
    assert_eq!(v(":").to_string(), ":");
    assert_eq!(v(":1.6").to_string(), ":1.6");
    assert_eq!(v("1.6:").to_string(), "1.6:");
}

#[test]
fn test_any_version() {
    assert_eq!(VersionUnion::List(any_version()), vl(&[":"]));
    assert_in(&v("1.2.5:1.2.7"), &VersionUnion::List(any_version()));
}

#[test]
fn test_concrete_and_concrete_range_as_version() {
    let single_range = match vl(&["1.2"]) {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    // "1.2" parses as the range 1.2:1.2, which is not concrete...
    assert!(single_range.concrete().is_none());
    // ... but collapses to the version 1.2 for old-Spack compatibility.
    assert_eq!(
        single_range.concrete_range_as_version(),
        Some(VersionItem::Version(sv("1.2")))
    );

    let wide_range = match vl(&["1.2:1.4"]) {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    assert!(wide_range.concrete_range_as_version().is_none());

    let concrete = match vl(&["=1.2"]) {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    assert_eq!(concrete.concrete(), Some(&VersionItem::Version(sv("1.2"))));
}

#[test]
fn test_intersect_in_place() {
    let mut l = match vl(&["1.0:2.0"]) {
        VersionUnion::List(l) => l,
        _ => panic!("expected a list"),
    };
    assert!(l.intersect(&v("1.5:3")).unwrap());
    assert_eq!(VersionUnion::List(l.clone()), vl(&["1.5:2.0"]));
    assert!(!l.intersect(&v(":")).unwrap());
}

#[test]
fn test_python_parity_edge_cases() {
    // Outputs verified against the Python implementation on 2026-08-22.

    // Lazily stringified results of the set algebra.
    let isect = v("1.0:2.7").intersection(&v("2.5:3.0")).unwrap();
    assert!(matches!(isect, VersionUnion::Range(_)));
    assert_eq!(isect.to_string(), "2.5:2.7");

    // Union of disjoint concrete versions is a sorted list.
    let u = v("=1.4").union(&v("=1.2")).unwrap();
    assert!(matches!(u, VersionUnion::List(_)));
    assert_eq!(u.to_string(), "=1.2,=1.4");

    // A range unioned with a contained git version stays the range.
    let ug = v("1:1.0").union(&v("ref=1.0")).unwrap();
    assert!(matches!(ug, VersionUnion::Range(_)));
    assert_eq!(ug.to_string(), "1:1.0");

    // Python: StandardVersion.union(GitVersion) raises NotImplementedError.
    assert!(matches!(
        v("=1.0").union(&v("ref=1.0")),
        Err(VersionError::Type(_))
    ));

    // Successor/predecessor logic, including infinity versions.
    assert_eq!(next_version(&sv("1.2a")).to_string(), "1.2b");
    assert_eq!(next_version(&sv("master")).to_string(), "main");
    assert_eq!(prev_version(&next_version(&sv("main"))).to_string(), "main");
    assert_eq!(next_version(&sv("1.2alpha3")).to_string(), "1.2alpha4");

    // Quirk: "final" is accepted as a prerelease marker, so 1.0final == 1.0.
    assert_eq!(sv("1.0final"), sv("1.0"));
    assert!(!sv("1.0final").is_prerelease());

    // String-component ranges.
    assert_eq!(
        version_range("develop", "develop").unwrap().to_string(),
        "develop"
    );

    // Prerelease ranges exclude later prerelease kinds.
    assert!(!v("=1.2.0rc1")
        .satisfies(&v("1.2.0alpha:1.2.0beta"))
        .unwrap());
}

#[test]
fn test_up_to_zero_and_empty_version() {
    // Versions used as elements stay meaningful even with no components.
    assert_eq!(sv("1.2").up_to(0), sv(""));
    assert_eq!(sv("").len(), 0);
}
