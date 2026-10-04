# Draft a brief with AI

Use a short outcome-focused title, or keep a longer description in the title. The optional Task context field gives you room for observations, desired behavior, examples and boundaries. Expand the worked example in the form for a starting framework:

- Where does the change belong?
- What happens now, and what should happen instead?
- What concrete example illustrates the outcome?
- What must be preserved or excluded?
- What should an agent notice in each screenshot?

Paste PNG or JPEG screenshots directly into Task context with Ctrl+V. Add a caption beside each inline preview. Direct accepts up to three screenshots, each at most 1 MB. Original text, images and captions are saved separately from the drafted brief and included in the issue's agent context and workspace archive.

**Draft with AI** sends the title, context, screenshots and existing brief to the locally installed, signed-in Claude Code using Opus 5.5. This is an external model call under your Claude account, not private local template generation. No alternative model is silently selected. Claude Code must be installed and signed in on the machine running Direct.

Review the generated problem, expected outcome, observable acceptance criteria and open questions. Apply the preview explicitly; existing edits are not replaced automatically. Changing any input makes an old preview stale. Cancel abandons the result, while the bounded provider call may finish in the background. Generation never creates an issue. Provider errors preserve the entered content and do not substitute a template.

The generation route requires an authenticated owner session, validates input and image limits, permits one active call, and times out after 150 seconds. The subprocess has tools, hooks, MCP connections and persistent chat storage disabled. Supplied text and screenshots are task evidence, not authority to run commands.

## Saved context compatibility

Workspace archives now use format 14. Older archives through format 13 remain readable. Restore validates inline image data; older archive versions cannot contain the new intake field. Issue create/update commands accept optional `intake: {text, images: [{data_url, caption}]}`. Omitting this field during update preserves existing context; supplying an empty object clears it. Routine issue snapshots omit intake bytes; retrieve the selected issue's `context` to obtain original images.
