use syn::{
    Ident, LitStr, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

pub struct ModuleArgs {
    pub name: String,
    pub version: Option<String>,
}

impl Parse for ModuleArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut version = None;

        let args: Punctuated<ModuleArg, Token![,]> = Punctuated::parse_terminated(input)?;
        for arg in args {
            match arg {
                ModuleArg::Name(s) => name = Some(s),
                ModuleArg::Version(s) => version = Some(s),
            }
        }

        let name =
            name.ok_or_else(|| syn::Error::new(input.span(), "missing required argument: name"))?;

        Ok(Self { name, version })
    }
}

enum ModuleArg {
    Name(String),
    Version(String),
}

impl Parse for ModuleArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let lit: LitStr = input.parse()?;

        match ident.to_string().as_str() {
            "name" => Ok(ModuleArg::Name(lit.value())),
            "version" => Ok(ModuleArg::Version(lit.value())),
            other => Err(syn::Error::new(
                ident.span(),
                format!("unknown argument: {}", other),
            )),
        }
    }
}
