// SPDX-License-Identifier: (Apache-2.0 OR MIT)

//! Golden-table tests for the lexer: the whole `spec_str, tokens` table of
//! `spec_syntax.py::test_parse_single_spec` (114 rows, including the `mock_git_test_package`
//! rows, whose token streams are static), the `test_spec_by_hash_tokens` rows, and the
//! tokenization-error rows of `test_error_reporting`.

use crate::parse::lexer::{tokenize, tokenize_all, TokenKind as K};

/// Expected token: kind, value, and the `virtuals`/`substitute` subvalues.
#[derive(Debug, PartialEq, Eq)]
struct Tok {
    kind: K,
    value: &'static str,
    virtuals: Option<&'static str>,
    substitute: Option<&'static str>,
}

fn t(kind: K, value: &'static str) -> Tok {
    Tok {
        kind,
        value,
        virtuals: None,
        substitute: None,
    }
}

fn tv(kind: K, value: &'static str, virtuals: &'static str, substitute: &'static str) -> Tok {
    Tok {
        kind,
        value,
        virtuals: Some(virtuals),
        substitute: Some(substitute),
    }
}

/// `SpecParser.tokens()`: the whitespace-filtered token list the golden table asserts against.
fn parser_tokens(input: &'static str) -> Vec<Tok> {
    tokenize(input)
        .unwrap_or_else(|e| panic!("tokenization error on {input:?}:\n{}", e.message()))
        .into_iter()
        .filter(|t| t.kind != K::Ws)
        .map(|t| Tok {
            kind: t.kind,
            value: t.value,
            virtuals: t.virtuals,
            substitute: t.substitute,
        })
        .collect()
}

const GOLDEN_ROWS: usize = 114;

fn golden() -> Vec<(&'static str, Vec<Tok>)> {
    vec![
        ("mvapich", vec![t(K::UnqualifiedPackageName, "mvapich")]),
        ("mvapich_foo", vec![t(K::UnqualifiedPackageName, "mvapich_foo")]),
        ("_mvapich_foo", vec![t(K::UnqualifiedPackageName, "_mvapich_foo")]),
        ("3dtk", vec![t(K::UnqualifiedPackageName, "3dtk")]),
        ("ns-3-dev", vec![t(K::UnqualifiedPackageName, "ns-3-dev")]),
        ("@2.7", vec![t(K::Version, "@2.7")]),
        ("@2.7:", vec![t(K::Version, "@2.7:")]),
        ("@:2.7", vec![t(K::Version, "@:2.7")]),
        ("+foo", vec![t(K::BoolVariant, "+foo")]),
        ("~foo", vec![t(K::BoolVariant, "~foo")]),
        ("-foo", vec![t(K::BoolVariant, "-foo")]),
        ("platform=test", vec![t(K::KeyValuePair, "platform=test")]),
        ("%intel", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel")]),
        ("languages=go @4.2:", vec![t(K::KeyValuePair, "languages=go"), t(K::Version, "@4.2:")]),
        ("@4.2:     languages=go", vec![t(K::Version, "@4.2:"), t(K::KeyValuePair, "languages=go")]),
        ("^zlib", vec![t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "zlib")]),
        ("openmpi ^hwloc", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc")]),
        ("openmpi ^hwloc ^libunwind", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "libunwind")]),
        ("openmpi      ^hwloc^libunwind", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "libunwind")]),
        ("foo @2.0 %bar@1.0", vec![t(K::UnqualifiedPackageName, "foo"), t(K::Version, "@2.0"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "bar"), t(K::Version, "@1.0")]),
        ("openmpi ^hwloc@1.2e6", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Version, "@1.2e6")]),
        ("openmpi ^hwloc@1.2e6:", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Version, "@1.2e6:")]),
        ("openmpi ^hwloc@:1.4b7-rc3", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Version, "@:1.4b7-rc3")]),
        ("openmpi ^hwloc@1.2e6:1.4b7-rc3", vec![t(K::UnqualifiedPackageName, "openmpi"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "hwloc"), t(K::Version, "@1.2e6:1.4b7-rc3")]),
        ("mvapich_foo ^_openmpi@1.2:1.4,1.6+debug~qt_4 %intel@12.1 ^stackwalker@8.1_1e", vec![t(K::UnqualifiedPackageName, "mvapich_foo"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "_openmpi"), t(K::Version, "@1.2:1.4,1.6"), t(K::BoolVariant, "+debug"), t(K::BoolVariant, "~qt_4"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel"), t(K::Version, "@12.1"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "stackwalker"), t(K::Version, "@8.1_1e")]),
        ("mvapich_foo ^_openmpi@1.2:1.4,1.6~qt_4 debug=2 %intel@12.1 ^stackwalker@8.1_1e", vec![t(K::UnqualifiedPackageName, "mvapich_foo"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "_openmpi"), t(K::Version, "@1.2:1.4,1.6"), t(K::BoolVariant, "~qt_4"), t(K::KeyValuePair, "debug=2"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel"), t(K::Version, "@12.1"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "stackwalker"), t(K::Version, "@8.1_1e")]),
        ("mvapich_foo ^_openmpi@1.2:1.4,1.6 cppflags=-O3 +debug~qt_4 %intel@12.1 ^stackwalker@8.1_1e", vec![t(K::UnqualifiedPackageName, "mvapich_foo"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "_openmpi"), t(K::Version, "@1.2:1.4,1.6"), t(K::KeyValuePair, "cppflags=-O3"), t(K::BoolVariant, "+debug"), t(K::BoolVariant, "~qt_4"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel"), t(K::Version, "@12.1"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "stackwalker"), t(K::Version, "@8.1_1e")]),
        ("yaml-cpp@0.1.8%intel@12.1 ^boost@3.1.4", vec![t(K::UnqualifiedPackageName, "yaml-cpp"), t(K::Version, "@0.1.8"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel"), t(K::Version, "@12.1"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "boost"), t(K::Version, "@3.1.4")]),
        ("builtin.yaml-cpp%gcc", vec![t(K::FullyQualifiedPackageName, "builtin.yaml-cpp"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc")]),
        ("testrepo.yaml-cpp%gcc", vec![t(K::FullyQualifiedPackageName, "testrepo.yaml-cpp"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc")]),
        ("builtin.yaml-cpp@0.1.8%gcc@7.2.0 ^boost@3.1.4", vec![t(K::FullyQualifiedPackageName, "builtin.yaml-cpp"), t(K::Version, "@0.1.8"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@7.2.0"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "boost"), t(K::Version, "@3.1.4")]),
        ("builtin.yaml-cpp ^testrepo.boost ^zlib", vec![t(K::FullyQualifiedPackageName, "builtin.yaml-cpp"), t(K::Dependency, "^"), t(K::FullyQualifiedPackageName, "testrepo.boost"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "zlib")]),
        ("mvapich ^stackwalker ^_openmpi", vec![t(K::UnqualifiedPackageName, "mvapich"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "stackwalker"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "_openmpi")]),
        ("y~f+e~d+c~b+a", vec![t(K::UnqualifiedPackageName, "y"), t(K::BoolVariant, "~f"), t(K::BoolVariant, "+e"), t(K::BoolVariant, "~d"), t(K::BoolVariant, "+c"), t(K::BoolVariant, "~b"), t(K::BoolVariant, "+a")]),
        ("@:", vec![t(K::Version, "@:")]),
        ("*", vec![t(K::UnqualifiedPackageName, "*")]),
        ("%foo=bar", vec![tv(K::Dependency, "%foo=bar", "foo", "bar")]),
        ("^foo=bar", vec![tv(K::Dependency, "^foo=bar", "foo", "bar")]),
        ("^*foo=bar", vec![t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "*"), t(K::KeyValuePair, "foo=bar")]),
        ("%*foo=bar", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "*"), t(K::KeyValuePair, "foo=bar")]),
        ("^*+foo", vec![t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "*"), t(K::BoolVariant, "+foo")]),
        ("^*~foo", vec![t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "*"), t(K::BoolVariant, "~foo")]),
        ("%*+foo", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "*"), t(K::BoolVariant, "+foo")]),
        ("%*~foo", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "*"), t(K::BoolVariant, "~foo")]),
        ("@1.6,1.2:1.4", vec![t(K::Version, "@1.6,1.2:1.4")]),
        ("os=fe", vec![t(K::KeyValuePair, "os=fe")]),
        ("os=default_os", vec![t(K::KeyValuePair, "os=default_os")]),
        ("target=be", vec![t(K::KeyValuePair, "target=be")]),
        ("target=default_target", vec![t(K::KeyValuePair, "target=default_target")]),
        ("platform=linux", vec![t(K::KeyValuePair, "platform=linux")]),
        ("develop-branch-version@abc12abc12abc12abc12abc12abc12abc12abc12=develop", vec![t(K::UnqualifiedPackageName, "develop-branch-version"), t(K::VersionHashPair, "@abc12abc12abc12abc12abc12abc12abc12abc12=develop")]),
        ("x ^y@foo ^y@foo", vec![t(K::UnqualifiedPackageName, "x"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::Version, "@foo"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::Version, "@foo")]),
        ("x ^y@foo ^y+bar", vec![t(K::UnqualifiedPackageName, "x"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::Version, "@foo"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::BoolVariant, "+bar")]),
        ("x ^y@foo +bar ^y@foo", vec![t(K::UnqualifiedPackageName, "x"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::Version, "@foo"), t(K::BoolVariant, "+bar"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "y"), t(K::Version, "@foo")]),
        ("_openmpi +debug-qt_4", vec![t(K::UnqualifiedPackageName, "_openmpi"), t(K::BoolVariant, "+debug-qt_4")]),
        ("_openmpi +debug -qt_4", vec![t(K::UnqualifiedPackageName, "_openmpi"), t(K::BoolVariant, "+debug"), t(K::BoolVariant, "-qt_4")]),
        ("_openmpi +debug~qt_4", vec![t(K::UnqualifiedPackageName, "_openmpi"), t(K::BoolVariant, "+debug"), t(K::BoolVariant, "~qt_4")]),
        ("target=:broadwell,icelake", vec![t(K::KeyValuePair, "target=:broadwell,icelake")]),
        ("develop-branch-version@git.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=develop+var1+var2", vec![t(K::UnqualifiedPackageName, "develop-branch-version"), t(K::VersionHashPair, "@git.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=develop"), t(K::BoolVariant, "+var1"), t(K::BoolVariant, "+var2")]),
        ("%gcc@10.2.1:", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@10.2.1:")]),
        ("%gcc@:10.2.1", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@:10.2.1")]),
        ("%gcc@10.2.1:12.1.0", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@10.2.1:12.1.0")]),
        ("%gcc@10.1.0,12.2.1:", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@10.1.0,12.2.1:")]),
        ("%gcc@:8.4.3,10.2.1:12.1.0", vec![t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@:8.4.3,10.2.1:12.1.0")]),
        ("dev_path=*", vec![t(K::KeyValuePair, "dev_path=*")]),
        ("dev_path=none", vec![t(K::KeyValuePair, "dev_path=none")]),
        ("dev_path=../relpath/work", vec![t(K::KeyValuePair, "dev_path=../relpath/work")]),
        ("dev_path=/abspath/work", vec![t(K::KeyValuePair, "dev_path=/abspath/work")]),
        ("cflags=a=b=c", vec![t(K::KeyValuePair, "cflags=a=b=c")]),
        ("cflags=a=b=c", vec![t(K::KeyValuePair, "cflags=a=b=c")]),
        ("cflags=a=b=c+~", vec![t(K::KeyValuePair, "cflags=a=b=c+~")]),
        ("cflags=-Wl,a,b,c", vec![t(K::KeyValuePair, "cflags=-Wl,a,b,c")]),
        ("cflags==\"-O3 -g\"", vec![t(K::PropagatedKeyValuePair, "cflags==\"-O3 -g\"")]),
        ("@1.2:1.4 , 1.6 ", vec![t(K::Version, "@1.2:1.4 , 1.6")]),
        ("a@1: b", vec![t(K::UnqualifiedPackageName, "a"), t(K::Version, "@1:"), t(K::UnqualifiedPackageName, "b")]),
        ("+ debug % intel @ 12.1:12.6", vec![t(K::BoolVariant, "+ debug"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "intel"), t(K::Version, "@ 12.1:12.6")]),
        ("@ 12.1:12.6 + debug - qt_4", vec![t(K::Version, "@ 12.1:12.6"), t(K::BoolVariant, "+ debug"), t(K::BoolVariant, "- qt_4")]),
        ("@10.4.0:10,11.3.0:target=aarch64:", vec![t(K::Version, "@10.4.0:10,11.3.0:"), t(K::KeyValuePair, "target=aarch64:")]),
        ("@:0.4 % nvhpc", vec![t(K::Version, "@:0.4"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "nvhpc")]),
        ("^[virtuals=mpi] openmpi", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "virtuals=mpi"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "openmpi")]),
        ("^mpi=openmpi", vec![tv(K::Dependency, "^mpi=openmpi", "mpi", "openmpi")]),
        ("^[virtuals=mpi] openmpi+foo ^[virtuals=lapack] openmpi+bar", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "virtuals=mpi"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "openmpi"), t(K::BoolVariant, "+foo"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "virtuals=lapack"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "openmpi"), t(K::BoolVariant, "+bar")]),
        ("^lapack,mpi=openmpi+foo+bar", vec![tv(K::Dependency, "^lapack,mpi=openmpi", "lapack,mpi", "openmpi"), t(K::BoolVariant, "+foo"), t(K::BoolVariant, "+bar")]),
        ("^[deptypes=link,build] zlib", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=link,build"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib")]),
        ("^[deptypes=link] zlib ^[deptypes=build] zlib", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=build"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib")]),
        ("^[deptypes=build,link] zlib ^[deptypes=link] zlib", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=build,link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib")]),
        ("pkg-a ^pkg-b %pkg-c ^pkg-b", vec![t(K::UnqualifiedPackageName, "pkg-a"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "pkg-c"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b")]),
        ("pkg-a ^pkg-b ^pkg-b %pkg-c", vec![t(K::UnqualifiedPackageName, "pkg-a"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "pkg-c")]),
        ("pkg-a ^pkg-b ^pkg-b@1 %pkg-c", vec![t(K::UnqualifiedPackageName, "pkg-a"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Version, "@1"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "pkg-c")]),
        ("pkg-a ^pkg-b@1 ^pkg-b %pkg-c", vec![t(K::UnqualifiedPackageName, "pkg-a"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Version, "@1"), t(K::Dependency, "^"), t(K::UnqualifiedPackageName, "pkg-b"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "pkg-c")]),
        ("^[deptypes=link] zlib ^[deptypes=run] zlib", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=run"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib")]),
        ("^[deptypes=build,link] zlib ^[deptypes=link] zlib", vec![t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=build,link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "deptypes=link"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "zlib")]),
        ("git-test@git.foo/bar", vec![t(K::UnqualifiedPackageName, "git-test"), t(K::GitVersion, "@git.foo/bar")]),
        ("zlib ++foo", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::PropagatedBoolVariant, "++foo")]),
        ("zlib ~~foo", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::PropagatedBoolVariant, "~~foo")]),
        ("zlib foo==bar", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::PropagatedKeyValuePair, "foo==bar")]),
        ("zlib %[virtuals=c] gcc", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "%["), t(K::KeyValuePair, "virtuals=c"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc")]),
        ("zlib %c=gcc", vec![t(K::UnqualifiedPackageName, "zlib"), tv(K::Dependency, "%c=gcc", "c", "gcc")]),
        ("zlib %[virtuals=c,cxx] gcc", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "%["), t(K::KeyValuePair, "virtuals=c,cxx"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc")]),
        ("zlib %c,cxx=gcc", vec![t(K::UnqualifiedPackageName, "zlib"), tv(K::Dependency, "%c,cxx=gcc", "c,cxx", "gcc")]),
        ("zlib %[virtuals=c,cxx] gcc@14.1", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "%["), t(K::KeyValuePair, "virtuals=c,cxx"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@14.1")]),
        ("zlib %c,cxx=gcc@14.1", vec![t(K::UnqualifiedPackageName, "zlib"), tv(K::Dependency, "%c,cxx=gcc", "c,cxx", "gcc"), t(K::Version, "@14.1")]),
        ("zlib %[virtuals=fortran] gcc@14.1 %[virtuals=c,cxx] clang", vec![t(K::UnqualifiedPackageName, "zlib"), t(K::StartEdgeProperties, "%["), t(K::KeyValuePair, "virtuals=fortran"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc"), t(K::Version, "@14.1"), t(K::StartEdgeProperties, "%["), t(K::KeyValuePair, "virtuals=c,cxx"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "clang")]),
        ("zlib %fortran=gcc@14.1 %c,cxx=clang", vec![t(K::UnqualifiedPackageName, "zlib"), tv(K::Dependency, "%fortran=gcc", "fortran", "gcc"), t(K::Version, "@14.1"), tv(K::Dependency, "%c,cxx=clang", "c,cxx", "clang")]),
        ("gcc languages:=c,c++", vec![t(K::UnqualifiedPackageName, "gcc"), t(K::KeyValuePair, "languages:=c,c++")]),
        ("gcc languages:==c,c++", vec![t(K::UnqualifiedPackageName, "gcc"), t(K::PropagatedKeyValuePair, "languages:==c,c++")]),
        ("mvapich %gcc languages:=c,c++ target=x86_64", vec![t(K::UnqualifiedPackageName, "mvapich"), t(K::Dependency, "%"), t(K::UnqualifiedPackageName, "gcc"), t(K::KeyValuePair, "languages:=c,c++"), t(K::KeyValuePair, "target=x86_64")]),
        ("foo ^[when='%c' virtuals=c] gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "when='%c'"), t(K::KeyValuePair, "virtuals=c"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc")]),
        ("foo ^[when='%c' virtuals=c]gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "when='%c'"), t(K::KeyValuePair, "virtuals=c"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc")]),
        ("foo ^[when='%c'] c=gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::StartEdgeProperties, "^["), t(K::KeyValuePair, "when='%c'"), tv(K::EndEdgeProperties, "] c=gcc", "c", "gcc")]),
        ("foo %%gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::Dependency, "%%"), t(K::UnqualifiedPackageName, "gcc")]),
        ("foo %%c,cxx=gcc", vec![t(K::UnqualifiedPackageName, "foo"), tv(K::Dependency, "%%c,cxx=gcc", "c,cxx", "gcc")]),
        ("foo %%[when='%c'] c=gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::StartEdgeProperties, "%%["), t(K::KeyValuePair, "when='%c'"), tv(K::EndEdgeProperties, "] c=gcc", "c", "gcc")]),
        ("foo %%[when='%c' virtuals=c] gcc", vec![t(K::UnqualifiedPackageName, "foo"), t(K::StartEdgeProperties, "%%["), t(K::KeyValuePair, "when='%c'"), t(K::KeyValuePair, "virtuals=c"), t(K::EndEdgeProperties, "]"), t(K::UnqualifiedPackageName, "gcc")]),
    ]
}

#[test]
fn test_parse_single_spec_golden_table() {
    let rows = golden();
    assert_eq!(rows.len(), GOLDEN_ROWS);
    for (input, expected) in rows {
        assert_eq!(
            parser_tokens(input),
            expected,
            "token stream mismatch for {input:?}"
        );
    }
}

#[test]
fn test_spec_by_hash_tokens() {
    assert_eq!(parser_tokens("/abcde"), vec![t(K::DagHash, "/abcde")]);
    assert_eq!(
        parser_tokens("foo/abcde"),
        vec![t(K::UnqualifiedPackageName, "foo"), t(K::DagHash, "/abcde")]
    );
    assert_eq!(
        parser_tokens("foo@1.2.3 /abcde"),
        vec![
            t(K::UnqualifiedPackageName, "foo"),
            t(K::Version, "@1.2.3"),
            t(K::DagHash, "/abcde"),
        ]
    );
}

/// The `test_error_reporting` rows: tokenization errors and their caret underlines.
#[test]
fn test_error_reporting_underlines() {
    let rows: &[(&str, &str)] = &[
        ("x@@1.2", "x@@1.2\n ^"),
        ("y ^x@@1.2", "y ^x@@1.2\n    ^"),
        ("x@1.2::", "x@1.2::\n      ^"),
        ("x::", "x::\n ^^"),
        (
            "cflags=''-Wl,a,b,c''",
            "cflags=''-Wl,a,b,c''\n            ^ ^ ^ ^^",
        ),
        (
            "@1.2:   develop   = foo",
            "@1.2:   develop   = foo\n                  ^^",
        ),
        (
            "@1.2:develop   = foo",
            "@1.2:develop   = foo\n               ^^",
        ),
    ];
    for (input, expected) in rows {
        let err = tokenize(input).expect_err("expected a tokenization error");
        let rendered = format!("{}\n{}", input, err.underline());
        assert!(
            rendered.starts_with(expected),
            "underline mismatch for {input:?}:\n{rendered}\nexpected prefix:\n{expected}"
        );
    }
}

/// The raw scanner covers the whole input contiguously, `WS` and `UNEXPECTED` included.
#[test]
fn test_tokenize_all_covers_input() {
    for input in [
        "",
        "x  y",
        "^ %% ]] @@ ..",
        "a b\tc\nd",
        "'lone quote",
        "x@1.2:   = foo",
    ] {
        let tokens = tokenize_all(input);
        let mut pos = 0;
        let mut chars = 0;
        let mut rebuilt = String::new();
        for token in &tokens {
            assert_eq!(token.start, chars, "gap before token in {input:?}");
            pos += token.value.len();
            chars = token.end;
            rebuilt.push_str(token.value);
        }
        assert_eq!(pos, input.len());
        assert_eq!(rebuilt, *input);
    }
}
