//! Expansion logic for the `#[derive(AuditView)]` macro.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Attribute, DeriveInput, Field, LitStr, parse_macro_input};

const AUTO_SKIP_FIELDS: &[&str] = &["created_at", "updated_at", "deleted_at", "deactivated_at"];

/// Expand the `AuditView` derive macro.
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_inner(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_inner(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let struct_attrs = parse_struct_attrs(&input.attrs)?;
    let target_type = struct_attrs
        .target_type
        .ok_or_else(|| syn::Error::new_spanned(input, "missing #[audit(target_type = \"...\")]"))?;
    let id_field = struct_attrs.id_field.unwrap_or_else(|| format_ident!("id"));
    let display_field = struct_attrs.display_field;

    let fields = match &input.data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "AuditView requires a named-field struct",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "AuditView requires a struct",
            ));
        }
    };

    let projections = fields
        .iter()
        .filter_map(|f| field_projection(f).transpose())
        .collect::<syn::Result<Vec<_>>>()?;

    let display_expr = match &display_field {
        Some(name) => quote!(Some(::std::string::ToString::to_string(&self.#name))),
        None if fields
            .iter()
            .any(|f| f.ident.as_ref().is_some_and(|i| i == "name")) =>
        {
            quote!(Some(::std::string::ToString::to_string(&self.name)))
        }
        None => quote!(None),
    };

    let name = &input.ident;
    let target_type_lit = LitStr::new(&target_type, proc_macro2::Span::call_site());

    let truncatable_keys = fields
        .iter()
        .filter_map(|f| {
            let ident = f.ident.as_ref()?;
            let mut is_truncatable = false;
            for attr in f.attrs.iter().filter(|a| a.path().is_ident("audit")) {
                // Ignore parse errors here; attribute validation is done in field_projection.
                let _res = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("truncatable") {
                        is_truncatable = true;
                    }
                    Ok(())
                });
            }
            if is_truncatable {
                Some(LitStr::new(
                    &ident.to_string(),
                    proc_macro2::Span::call_site(),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<LitStr>>();

    Ok(quote! {
        impl ::uptrakit_audit_log::AuditView for #name {
            const TARGET_TYPE: &'static str = #target_type_lit;

            fn audit_target_id(&self) -> ::std::string::String {
                ::std::string::ToString::to_string(&self.#id_field)
            }

            fn audit_target_display(&self) -> ::std::option::Option<::std::string::String> {
                #display_expr
            }

            fn audit_view(&self) -> ::serde_json::Value {
                let mut map = ::serde_json::Map::new();
                #(#projections)*
                ::serde_json::Value::Object(map)
            }

            fn audit_truncatable_fields() -> &'static [&'static str] {
                &[#(#truncatable_keys),*]
            }
        }
    })
}

struct StructAttrs {
    target_type: Option<String>,
    id_field: Option<syn::Ident>,
    display_field: Option<syn::Ident>,
}

fn parse_struct_attrs(attrs: &[Attribute]) -> syn::Result<StructAttrs> {
    let mut out = StructAttrs {
        target_type: None,
        id_field: None,
        display_field: None,
    };
    for attr in attrs.iter().filter(|a| a.path().is_ident("audit")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("target_type") {
                out.target_type = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("id_field") {
                out.id_field = Some(format_ident!(
                    "{}",
                    meta.value()?.parse::<LitStr>()?.value()
                ));
            } else if meta.path.is_ident("display_field") {
                out.display_field = Some(format_ident!(
                    "{}",
                    meta.value()?.parse::<LitStr>()?.value()
                ));
            } else {
                return Err(meta.error("unknown audit attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

enum FieldAction {
    Skip,
    Default,
    ProjectWith(syn::Ident),
}

fn field_projection(f: &Field) -> syn::Result<Option<TokenStream2>> {
    let ident = f
        .ident
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(f, "AuditView requires named struct fields"))?;
    let name_str = ident.to_string();
    let mut force_include = false;
    let mut action = FieldAction::Default;

    for attr in f.attrs.iter().filter(|a| a.path().is_ident("audit")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                action = FieldAction::Skip;
            } else if meta.path.is_ident("include") {
                force_include = true;
            } else if meta.path.is_ident("project_with") {
                let s: LitStr = meta.value()?.parse()?;
                action = FieldAction::ProjectWith(format_ident!("{}", s.value()));
            } else if meta.path.is_ident("truncatable") {
                // harvested separately for audit_truncatable_fields(); field still projected normally
            } else {
                return Err(meta.error("unknown audit field attribute"));
            }
            Ok(())
        })?;
    }

    if matches!(action, FieldAction::Skip) {
        return Ok(None);
    }
    if AUTO_SKIP_FIELDS.contains(&name_str.as_str()) && !force_include {
        return Ok(None);
    }
    if name_str == "id" {
        return Ok(None);
    }

    let key_lit = LitStr::new(&name_str, proc_macro2::Span::call_site());
    Ok(Some(match action {
        FieldAction::ProjectWith(func) => quote! {
            map.insert(#key_lit.into(), #func(&self.#ident));
        },
        FieldAction::Default => quote! {
            map.insert(
                #key_lit.into(),
                ::serde_json::to_value(&self.#ident)
                    .unwrap_or(::serde_json::Value::Null),
            );
        },
        FieldAction::Skip => {
            // unreachable: Skip arm is handled by the early return above
            return Ok(None);
        }
    }))
}
