const SYSTEM_PROMPT: &str = include_str!("../prompts/codex_system_prompt.md");

pub const SYSTEM_PROMPT_FILE_NAME: &str = "codex_system_prompt.md";

pub fn system_prompt() -> &'static str {
    SYSTEM_PROMPT
}

pub fn render_task_prompt(user_prompt: &str) -> String {
    let system_prompt = system_prompt().trim();
    let user_prompt = user_prompt.trim();

    if user_prompt.is_empty() {
        format!("{system_prompt}\n")
    } else {
        format!("{system_prompt}\n\n## User Request\n\n{user_prompt}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{render_task_prompt, system_prompt};

    #[test]
    fn renders_combined_prompt_with_user_request_section() {
        let rendered = render_task_prompt("Draft the reply.");

        assert!(rendered.contains("## Workspace contract"));
        assert!(rendered.contains("## User Request"));
        assert!(rendered.contains("Draft the reply."));
    }

    #[test]
    fn exposes_system_prompt_text() {
        let prompt = system_prompt();

        assert!(prompt.contains("reply_email_draft.html"));
        assert!(prompt.contains(".agents/skills/"));
    }
}
