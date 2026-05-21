#![allow(missing_docs)]

mod args {
    #![allow(missing_docs)]

    use super::super::super::grok_cli::*;

    #[test]
    fn grok_args_pass_model_with_long_flag() {
        let transport = GrokCliTransport::new(Some("grok-build".to_string()));

        assert_eq!(transport.args(), vec!["--model", "grok-build"]);
    }
}
