#![allow(missing_docs)]

// Tests for extract/csharp.rs live here as sibling under extract/tests/ per
// docs/design-patterns/test_layout.md. Explicit named imports, no blanket.

use super::super::csharp::CSharpExtractor;
use super::super::{ExtractedLeaf, FileExtractor, FileKind, Language};

fn fixture() -> &'static str {
    r#"using System;

namespace Orbit.Sample
{
    public delegate void AccountChanged(object sender, EventArgs args);

    public interface IAccountRepository
    {
        event EventHandler Loaded;
        string Name { get; }
        void Save(Account account);
    }

    public enum AccountStatus
    {
        Active,
        Suspended
    }

    public struct AccountKey
    {
        public Guid Value { get; }
    }

    public record AccountSnapshot(string Id, AccountStatus Status);

    public class AccountService : IAccountRepository
    {
        private readonly string _prefix = "acct";
        public event EventHandler Changed;
        public event EventHandler Loaded { add { } remove { } }
        public string Name { get; private set; }

        public Account CreateAccount(string id)
        {
            return new Account(id);
        }
    }
}"#
}

fn leaf<'a>(leaves: &'a [ExtractedLeaf], qualified_name: &str, kind: &str) -> &'a ExtractedLeaf {
    leaves
        .iter()
        .find(|leaf| leaf.qualified_name == qualified_name && leaf.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} leaf {qualified_name} in {leaves:#?}"))
}

#[test]
fn file_kind_is_csharp() {
    assert_eq!(
        CSharpExtractor.file_kind(),
        FileKind::Code(Language::CSharp)
    );
}

#[test]
fn extracts_required_csharp_symbols() {
    let result = CSharpExtractor.extract(fixture());
    let leaves = result.leaves;

    let namespace = leaf(&leaves, "Orbit.Sample", "namespace");
    assert_eq!(namespace.name, "Orbit.Sample");
    assert_eq!(namespace.start_line, 3);
    assert_eq!(namespace.end_line, 39);
    assert!(
        namespace
            .children_qualified_names
            .contains(&"Orbit.Sample::AccountService".to_string())
    );

    let delegate = leaf(&leaves, "Orbit.Sample::AccountChanged", "delegate");
    assert_eq!(delegate.name, "AccountChanged");
    assert_eq!(delegate.start_line, 5);
    assert!(
        delegate
            .source
            .trim_start()
            .starts_with("public delegate void")
    );

    let interface = leaf(&leaves, "Orbit.Sample::IAccountRepository", "interface");
    assert_eq!(interface.start_line, 7);
    assert_eq!(interface.end_line, 12);

    let interface_event = leaf(&leaves, "Orbit.Sample::IAccountRepository::Loaded", "event");
    assert_eq!(interface_event.start_line, 9);
    assert_eq!(
        interface_event.parent_qualified_name.as_deref(),
        Some("Orbit.Sample::IAccountRepository")
    );

    let interface_property = leaf(
        &leaves,
        "Orbit.Sample::IAccountRepository::Name",
        "property",
    );
    assert_eq!(interface_property.start_line, 10);

    let interface_method = leaf(&leaves, "Orbit.Sample::IAccountRepository::Save", "method");
    assert_eq!(interface_method.start_line, 11);
    assert!(
        interface_method
            .source
            .trim_start()
            .starts_with("void Save")
    );

    let enum_leaf = leaf(&leaves, "Orbit.Sample::AccountStatus", "enum");
    assert_eq!(enum_leaf.start_line, 14);
    assert_eq!(enum_leaf.end_line, 18);

    let struct_leaf = leaf(&leaves, "Orbit.Sample::AccountKey", "struct");
    assert_eq!(struct_leaf.start_line, 20);
    assert_eq!(struct_leaf.end_line, 23);

    let struct_property = leaf(&leaves, "Orbit.Sample::AccountKey::Value", "property");
    assert_eq!(struct_property.start_line, 22);

    let record_leaf = leaf(&leaves, "Orbit.Sample::AccountSnapshot", "record");
    assert_eq!(record_leaf.start_line, 25);
    assert_eq!(record_leaf.end_line, 25);

    let class_leaf = leaf(&leaves, "Orbit.Sample::AccountService", "class");
    assert_eq!(class_leaf.start_line, 27);
    assert_eq!(class_leaf.end_line, 38);
    assert!(
        class_leaf
            .children_qualified_names
            .contains(&"Orbit.Sample::AccountService::CreateAccount".to_string())
    );

    let field = leaf(&leaves, "Orbit.Sample::AccountService::_prefix", "field");
    assert_eq!(field.start_line, 29);
    assert!(
        field
            .source
            .trim_start()
            .starts_with("private readonly string")
    );

    let event_field = leaf(&leaves, "Orbit.Sample::AccountService::Changed", "event");
    assert_eq!(event_field.start_line, 30);

    let event_property = leaf(&leaves, "Orbit.Sample::AccountService::Loaded", "event");
    assert_eq!(event_property.start_line, 31);

    let property = leaf(&leaves, "Orbit.Sample::AccountService::Name", "property");
    assert_eq!(property.start_line, 32);
    assert!(
        property
            .source
            .trim_start()
            .starts_with("public string Name")
    );

    let method = leaf(
        &leaves,
        "Orbit.Sample::AccountService::CreateAccount",
        "method",
    );
    assert_eq!(method.start_line, 34);
    assert_eq!(method.end_line, 37);
    assert!(
        method
            .source
            .trim_start()
            .starts_with("public Account CreateAccount")
    );
}

#[test]
fn file_scoped_namespace_parents_following_declarations() {
    let source = r#"namespace Orbit.FileScoped;

public class FileScopedService
{
    public string Name { get; }
}"#;

    let result = CSharpExtractor.extract(source);
    let leaves = result.leaves;

    let namespace = leaf(&leaves, "Orbit.FileScoped", "namespace");
    assert_eq!(namespace.start_line, 1);
    assert_eq!(namespace.end_line, 1);

    let class = leaf(&leaves, "Orbit.FileScoped::FileScopedService", "class");
    assert_eq!(
        class.parent_qualified_name.as_deref(),
        Some("Orbit.FileScoped")
    );

    let property = leaf(
        &leaves,
        "Orbit.FileScoped::FileScopedService::Name",
        "property",
    );
    assert_eq!(
        property.parent_qualified_name.as_deref(),
        Some("Orbit.FileScoped::FileScopedService")
    );
}
