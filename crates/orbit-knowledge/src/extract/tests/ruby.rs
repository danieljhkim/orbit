#![allow(missing_docs)]

// Tests for extract/ruby.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::ruby::RubyExtractor;
use super::super::{ExtractedLeaf, FileExtractor, FileKind, Language};

fn fixture() -> &'static str {
    r#"APP_ROOT = "/srv"

module Billing
  class Invoice
    attr_accessor :total
    attr_reader :status
    attr_writer :token

    def initialize(total)
      @total = total
    end

    def self.from_hash(hash)
      new(hash[:total])
    end

    class << self
      def reset_cache
      end
    end
  end
end"#
}

fn leaf<'a>(leaves: &'a [ExtractedLeaf], name: &str, kind: &str) -> &'a ExtractedLeaf {
    leaves
        .iter()
        .find(|leaf| leaf.name == name && leaf.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} leaf {name}"))
}

#[test]
fn file_kind_is_ruby() {
    assert_eq!(RubyExtractor.file_kind(), FileKind::Code(Language::Ruby));
}

#[test]
fn extracts_ruby_declarations_and_runtime_accessors() {
    let result = RubyExtractor.extract(fixture());
    let leaves = result.leaves;

    let constant = leaf(&leaves, "APP_ROOT", "constant");
    assert_eq!(constant.start_line, 1);
    assert_eq!(constant.end_line, 1);

    let module = leaf(&leaves, "Billing", "module");
    assert_eq!(module.qualified_name, "Billing");
    assert_eq!(module.start_line, 3);
    assert_eq!(module.end_line, 22);
    assert!(
        module
            .children_qualified_names
            .contains(&"Billing::Invoice".to_string())
    );

    let class = leaf(&leaves, "Invoice", "class");
    assert_eq!(class.qualified_name, "Billing::Invoice");
    assert_eq!(class.parent_qualified_name.as_deref(), Some("Billing"));
    assert_eq!(class.start_line, 4);
    assert_eq!(class.end_line, 21);

    let initialize = leaf(&leaves, "initialize", "method");
    assert_eq!(initialize.qualified_name, "Billing::Invoice::initialize");
    assert_eq!(
        initialize.parent_qualified_name.as_deref(),
        Some("Billing::Invoice")
    );
    assert_eq!(initialize.start_line, 9);
    assert_eq!(initialize.end_line, 11);

    let singleton_method = leaf(&leaves, "self.from_hash", "singleton_method");
    assert_eq!(
        singleton_method.qualified_name,
        "Billing::Invoice.from_hash"
    );
    assert_eq!(
        singleton_method.parent_qualified_name.as_deref(),
        Some("Billing::Invoice")
    );
    assert_eq!(singleton_method.start_line, 13);
    assert_eq!(singleton_method.end_line, 15);

    let singleton_class = leaf(&leaves, "self", "singleton_class");
    assert_eq!(singleton_class.qualified_name, "Billing::Invoice::self");
    assert_eq!(singleton_class.start_line, 17);
    assert_eq!(singleton_class.end_line, 20);

    let reader = leaf(&leaves, "total", "method");
    assert_eq!(reader.qualified_name, "Billing::Invoice::total");
    assert_eq!(reader.start_line, 5);
    assert_eq!(reader.end_line, 5);
    assert_eq!(reader.source, "attr_accessor :total");

    let accessor_writer = leaf(&leaves, "total=", "method");
    assert_eq!(accessor_writer.qualified_name, "Billing::Invoice::total=");
    assert_eq!(accessor_writer.start_line, 5);
    assert_eq!(accessor_writer.end_line, 5);

    let attr_reader = leaf(&leaves, "status", "method");
    assert_eq!(attr_reader.start_line, 6);
    assert_eq!(attr_reader.end_line, 6);

    let attr_writer = leaf(&leaves, "token=", "method");
    assert_eq!(attr_writer.start_line, 7);
    assert_eq!(attr_writer.end_line, 7);
}

#[test]
fn ignores_explicit_receiver_attr_calls() {
    let result = RubyExtractor.extract(
        r#"class Account
  Other.attr_accessor :token
end"#,
    );

    assert!(
        result
            .leaves
            .iter()
            .all(|leaf| leaf.name != "token" && leaf.name != "token=")
    );
}
