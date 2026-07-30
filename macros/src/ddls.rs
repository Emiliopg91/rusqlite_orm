use std::fs;

use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input};

pub fn dlls(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);

    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(d) => d,
        Err(_) => {
            return syn::Error::new(lit.span(), "CARGO_MANIFEST_DIR not set")
                .to_compile_error()
                .into();
        }
    };
    let ddls_dir = std::path::Path::new(&manifest_dir)
        .join(lit.value())
        .display()
        .to_string();

    let read_dir = match fs::read_dir(&ddls_dir) {
        Ok(rd) => rd,
        Err(e) => {
            return syn::Error::new(
                lit.span(),
                format!("could not read directory `{ddls_dir}`: {e}"),
            )
            .to_compile_error()
            .into();
        }
    };

    let files = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sql").unwrap_or(false))
        .collect::<Vec<_>>();

    let mut entries = Vec::with_capacity(files.len());

    for path in &files {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!("invalid file path: {}", path.display()),
                )
                .to_compile_error()
                .into();
            }
        };

        let version = match name.split('_').next().and_then(|v| v.parse::<u16>().ok()) {
            Some(v) => v,
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!(
                        "invalid DDL file name `{name}`: expected format `<version>_<name>.sql`"
                    ),
                )
                .to_compile_error()
                .into();
            }
        };

        let mut content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return syn::Error::new(
                    lit.span(),
                    format!("could not read `{}`: {e}", path.display()),
                )
                .to_compile_error()
                .into();
            }
        };

        let description = match content
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("--"))
        {
            Some(d) => d.trim().to_string(),
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!("missing description comment in `{name}`"),
                )
                .to_compile_error()
                .into();
            }
        };

        content = content
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with("--") {
                    None
                } else {
                    Some(l.to_string())
                }
            })
            .collect::<Vec<String>>()
            .join("\n");

        entries.push(quote! {
            rusqlite_orm::database::DdlVersion {
                version: #version,
                description: #description,
                sql: #content,
            }
        });
    }

    let len = entries.len();

    quote! {
        pub static DDLS: [rusqlite_orm::database::DdlVersion; #len] = [
            #(#entries),*
        ];
    }
    .into()
}
