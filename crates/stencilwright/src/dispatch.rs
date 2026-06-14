//! Command dispatch for site-first resource operations.

use anyhow::{Result, bail};
use clap::Parser;
use stencil_core::UnmaskApprovalContext;
use stencil_places::recognize;

use crate::cli::{
    ConfigCommand, PageCommand, PlaceCommand, PlaceElementCommand, PlaceTargetCli,
    PlaceTargetCommand, SiteCli, SiteCommand, SiteElementCommand, ValueCommand,
};
use crate::{browser, config, elements, page_cmd, place_edit, places, session, values};

pub(crate) async fn site(args: Vec<String>) -> Result<()> {
    dispatch_site(parse_site(args)?).await
}

async fn dispatch_site(cli: SiteCli) -> Result<()> {
    match cli.command {
        SiteCommand::Config { op } => dispatch_config(&cli.site, op),
        SiteCommand::Place { op } => dispatch_place(&cli.site, op).await,
        SiteCommand::Element { op } => dispatch_site_element(&cli.site, op).await,
        SiteCommand::Page { op } => dispatch_page(&cli.site, op).await,
        SiteCommand::Value { op } => dispatch_value(&cli.site, op).await,
    }
}

fn dispatch_config(site: &str, op: ConfigCommand) -> Result<()> {
    let site_dir = session::site_dir(site);
    match op {
        ConfigCommand::Show => config::show(&site_dir),
        ConfigCommand::Set(args) => config::set(&site_dir, args),
    }
}

async fn dispatch_place(site: &str, op: PlaceCommand) -> Result<()> {
    match op {
        PlaceCommand::Add { name, selector } => places::add(site, &name, selector.as_deref()).await,
        PlaceCommand::List => places::list(site).await,
        PlaceCommand::Target(args) => {
            let target = parse_place_target(args)?;
            match target.command {
                PlaceTargetCommand::Goto => places::goto(site, &target.name).await,
                PlaceTargetCommand::Set(args) => {
                    place_edit::set(&session::site_dir(site), &target.name, args)
                }
                PlaceTargetCommand::Signature { op } => match op {
                    crate::cli::SignatureCommand::Set(args) => {
                        place_edit::set_signature(&session::site_dir(site), &target.name, args)
                    }
                },
                PlaceTargetCommand::Completion { op } => match op {
                    crate::cli::CompletionCommand::Set(args) => {
                        place_edit::set_completion(&session::site_dir(site), &target.name, args)
                    }
                    crate::cli::CompletionCommand::Clear => {
                        place_edit::clear_completion(&session::site_dir(site), &target.name)
                    }
                },
                PlaceTargetCommand::Element { op } => {
                    dispatch_place_element(site, &target.name, op).await
                }
            }
        }
    }
}

async fn dispatch_place_element(site: &str, place: &str, op: PlaceElementCommand) -> Result<()> {
    let site_dir = session::site_dir(site);
    match op {
        PlaceElementCommand::Add(args) => {
            if args.unmasked
                && elements::needs_unmask(&site_dir, elements::Scope::Place(place), &args.selector)?
            {
                approve_at_place(
                    site,
                    place,
                    &args.selector,
                    args.name.as_deref(),
                    args.auto_fill.as_deref(),
                    args.reason.as_deref(),
                )
                .await?;
            }
            elements::add_place(
                &site_dir,
                place,
                elements::ElementInput {
                    selector: &args.selector,
                    name: args.name.as_deref(),
                    auto_fill: args.auto_fill.as_deref(),
                    unmasked: args.unmasked,
                },
            )
        }
        PlaceElementCommand::Unmask { selector, reason } => {
            if elements::needs_unmask(&site_dir, elements::Scope::Place(place), &selector)? {
                approve_at_place(site, place, &selector, None, None, reason.as_deref()).await?;
            }
            elements::unmask_place(&site_dir, place, &selector)
        }
        PlaceElementCommand::List => {
            let runtime = browser::load_site(site)?;
            elements::list_place(&runtime.graph, place)
        }
    }
}

async fn dispatch_site_element(site: &str, op: SiteElementCommand) -> Result<()> {
    let site_dir = session::site_dir(site);
    match op {
        SiteElementCommand::Add(args) => {
            if args.unmasked
                && elements::needs_unmask(&site_dir, elements::Scope::Site, &args.selector)?
            {
                let session = browser::attach(site).await?;
                let page = session.page();
                approve(
                    &page,
                    UnmaskApprovalContext {
                        scope: Some("site-wide".to_string()),
                        site: Some(site.to_string()),
                        reason: args.reason.clone(),
                        ..UnmaskApprovalContext::default()
                    },
                    &args.selector,
                    args.name.as_deref(),
                    args.auto_fill.as_deref(),
                )
                .await?;
            }
            elements::add_site(
                &site_dir,
                elements::ElementInput {
                    selector: &args.selector,
                    name: args.name.as_deref(),
                    auto_fill: args.auto_fill.as_deref(),
                    unmasked: args.unmasked,
                },
            )
        }
        SiteElementCommand::List => {
            let runtime = browser::load_site(site)?;
            elements::list_site(&runtime.graph);
            Ok(())
        }
    }
}

async fn approve_at_place(
    site: &str,
    place: &str,
    selector: &str,
    proposed_name: Option<&str>,
    auto_fill: Option<&str>,
    reason: Option<&str>,
) -> Result<()> {
    let runtime = browser::load_site(site)?;
    let session = browser::attach(site).await?;
    let page = session.page();
    let here = recognize::recognize(&runtime.graph, &page).await?;
    match here {
        Some(m) if m.place_name == place => {
            let place_def = runtime
                .graph
                .place(place)
                .ok_or_else(|| anyhow::anyhow!("unknown place: '{place}'"))?;
            approve(
                &page,
                UnmaskApprovalContext {
                    scope: Some(format!("place {place}")),
                    site: Some(site.to_string()),
                    place: Some(place.to_string()),
                    reason: reason.map(ToOwned::to_owned),
                    signature: Some(place_def.signature.clone()),
                    ..UnmaskApprovalContext::default()
                },
                selector,
                proposed_name,
                auto_fill,
            )
            .await
        }
        Some(m) => bail!(
            "live page is at '{}', not '{}'; run `{site} place {place} goto` first",
            m.place_name,
            place,
        ),
        None => bail!("live page is unrecognized; run `{site} place {place} goto` first"),
    }
}

async fn approve(
    page: &stencil_browser::Page,
    context: UnmaskApprovalContext,
    selector: &str,
    proposed_name: Option<&str>,
    auto_fill: Option<&str>,
) -> Result<()> {
    let decision = page
        .approve_unmask_with_context(&context, selector, proposed_name)
        .await?;
    if let Some(feedback) = &decision.feedback {
        println!("user feedback: {feedback}");
    }
    if decision.approved {
        Ok(())
    } else {
        let command = map_masked_command(&context, selector, proposed_name, auto_fill);
        bail!(
            "field not mapped; approval denied, so no unmask rule was written.\n\
             To map this field without unmasking, run:\n  {command}"
        )
    }
}

fn map_masked_command(
    context: &UnmaskApprovalContext,
    selector: &str,
    proposed_name: Option<&str>,
    auto_fill: Option<&str>,
) -> String {
    let site = context.site.as_deref().unwrap_or("<site>");
    let mut parts = vec!["stencilwright".to_string(), shell_quote(site)];
    if let Some(place) = context.place.as_deref().filter(|place| !place.is_empty()) {
        parts.extend([
            "place".to_string(),
            shell_quote(place),
            "element".to_string(),
            "add".to_string(),
            shell_quote(selector),
        ]);
    } else {
        parts.extend([
            "element".to_string(),
            "add".to_string(),
            shell_quote(selector),
        ]);
    }
    if let Some(name) = proposed_name.filter(|name| !name.is_empty()) {
        parts.extend(["--as".to_string(), shell_quote(name)]);
    }
    if let Some(auto_fill) = auto_fill.filter(|auto_fill| !auto_fill.is_empty()) {
        parts.extend(["--auto-fill".to_string(), shell_quote(auto_fill)]);
    }
    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

async fn dispatch_page(site: &str, op: PageCommand) -> Result<()> {
    match op {
        PageCommand::Goto { url } => page_cmd::goto(site, &url).await,
        PageCommand::Click { selector } => page_cmd::click(site, &selector).await,
        PageCommand::Fill { selector, value } => page_cmd::fill(site, &selector, &value).await,
        PageCommand::Dump => page_cmd::dump(site, &std::env::current_dir()?).await,
    }
}

async fn dispatch_value(site: &str, op: ValueCommand) -> Result<()> {
    let site_dir = session::site_dir(site);
    match op {
        ValueCommand::Add { name, reference } => values::add(&site_dir, &name, &reference),
        ValueCommand::Search(args) => values::search(site, args).await,
        ValueCommand::List => values::list(&site_dir),
        ValueCommand::Remove { name } => values::remove(&site_dir, &name),
    }
}

fn parse_site(args: Vec<String>) -> Result<SiteCli> {
    parse_from("stencilwright <SITE>", args)
}

fn parse_place_target(args: Vec<String>) -> Result<PlaceTargetCli> {
    parse_from("stencilwright <site> place", args)
}

fn parse_from<T: Parser>(bin: &str, args: Vec<String>) -> Result<T> {
    let argv = std::iter::once(bin.to_string())
        .chain(args)
        .collect::<Vec<_>>();
    match T::try_parse_from(argv) {
        Ok(parsed) => Ok(parsed),
        Err(e) => e.exit(),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_masked_command, shell_quote};
    use stencil_core::UnmaskApprovalContext;

    #[test]
    fn masked_mapping_command_preserves_place_scope() {
        let context = UnmaskApprovalContext {
            site: Some("example".to_string()),
            place: Some("listing_main".to_string()),
            ..UnmaskApprovalContext::default()
        };

        assert_eq!(
            map_masked_command(
                &context,
                "app-post a[slot=title]",
                Some("post_titles"),
                None
            ),
            "stencilwright example place listing_main element add 'app-post a[slot=title]' --as post_titles"
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a[slot='title']"), r#"'a[slot='\''title'\'']'"#);
    }
}
