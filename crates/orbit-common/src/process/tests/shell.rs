use super::super::shell::quote_posix_arg;

#[test]
fn quote_posix_arg_escapes_embedded_single_quotes() {
    assert_eq!(quote_posix_arg("plain"), "'plain'");
    assert_eq!(quote_posix_arg("a'b"), "'a'\\''b'");
    assert_eq!(quote_posix_arg("/srv/my ws"), "'/srv/my ws'");
}
