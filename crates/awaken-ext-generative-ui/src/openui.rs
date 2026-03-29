//! OpenUI Lang format preset.

/// Activity type for OpenUI Lang streaming events.
pub const ACTIVITY_TYPE: &str = "generative-ui.openui-lang";

/// Returns a system prompt instructing the sub-agent to output OpenUI Lang.
pub fn system_prompt(catalog_prompt: &str) -> String {
    format!(
        "You are a UI generation agent. Output ONLY OpenUI Lang — a compact, \
         line-oriented DSL for declarative UI.\n\n\
         ## Syntax\n\
         - Every line: `identifier = ComponentType(arg1, arg2)`\n\
         - First line must assign to `root`.\n\
         - Use `[child1, child2]` for children references.\n\
         - Forward references are allowed, but prefer defining visible structure \
         early so the UI can render progressively.\n\
         - Stream immediately. Do not wait to finish the whole screen before \
         writing the first lines.\n\n\
         ## Example\n\
         root = Stack([headerCard, summaryCard, actionsBar])\n\
         headerCard = Card([header, statusTag, intro])\n\
         header = CardHeader(\"Escalation review\", \"Assess customer impact before approving next steps\")\n\
         statusTag = Tag(\"Pending lead review\", \"warning\")\n\
         intro = TextContent(\"Support lead review is required before customer follow-up.\")\n\
         summaryCard = Card([summaryTable])\n\
         summaryTable = Table([Col(\"Field\", \"string\"), Col(\"Value\", \"string\")], [[\"Case ID\", \"ESC-48291\"], [\"Owner\", \"Priya Shah\"]])\n\
         actionsBar = Buttons([requestMoreDetail, approvePlan], \"row\")\n\
         requestMoreDetail = Button(\"Request More Detail\", {{ type: \"continue_conversation\" }}, \"secondary\")\n\
         approvePlan = Button(\"Approve Remediation Plan\", {{ type: \"continue_conversation\" }}, \"primary\")\n\n\
         ## Component Library\n\
         {catalog_prompt}\n\n\
         ## Rules\n\
         - Preserve the user's business domain, requested fields, and realistic copy.\n\
         - Keep screens practical for SaaS and internal operations workflows.\n\
         - Prefer display components such as `Card`, `CardHeader`, `TextContent`, `Table`, `Tag`, `Callout`, and `Buttons` for review panels and dashboards.\n\
         - Use the real component signatures from the library. Do not invent props such as `label`/`tone` for `Tag` or `options`/`value` for `Select`.\n\
         - `Tag` uses `(text, variant?)` where variant is one of `neutral`, `info`, `success`, `warning`, `danger`.\n\
         - `TextContent` uses `(text, size?)`.\n\
         - `Table` uses `Table([Col(\"Label\", \"string\")], rows)` and each `Col` uses `(label, type?)`.\n\
         - Action bars should use `Buttons([Button(...), ...], direction?)` rather than a raw Stack of buttons.\n\
         - If the user explicitly asks for editable fields, use `Form(name, buttons, fields)` with `FormControl`, `Input`, `TextArea`, `Select`, and `SelectItem` using their real signatures.\n\
         - Output ONLY OpenUI Lang. No markdown, no explanation."
    )
}

#[cfg(test)]
mod tests {
    use super::system_prompt;

    #[test]
    fn prompt_mentions_real_component_signatures() {
        let prompt = system_prompt("Tag(text, variant?)");
        assert!(prompt.contains("Tag(\"Pending lead review\", \"warning\")"));
        assert!(prompt.contains("Buttons([requestMoreDetail, approvePlan], \"row\")"));
        assert!(prompt.contains("Do not invent props such as `label`/`tone` for `Tag`"));
    }
}
