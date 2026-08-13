#![allow(missing_docs)]

mod args {
    #![allow(missing_docs)]

    use super::super::super::grok_cli::*;

    #[test]
    fn grok_args_pass_model_with_long_flag() {
        let transport = GrokCliTransport::new(Some("grok-4.6".to_string()));

        assert_eq!(transport.args(), vec!["--model", "grok-4.6"]);
    }
}
