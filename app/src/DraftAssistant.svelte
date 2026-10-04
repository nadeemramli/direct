<script lang="ts">
  import { onDestroy } from "svelte";
  import { draftBrief } from "./api";
  import type { DraftBriefResult, IntakeContext } from "./api";
  let { title, body = $bindable(), acceptance = $bindable(), intake = $bindable(), product, project, disabled = false }:
    {title:string;body:string;acceptance:string;intake:string;product:string;project:string;disabled?:boolean} = $props();
  let running = $state(false);
  let pasting = $state(false);
  let message = $state("");
  let result = $state<DraftBriefResult | null>(null);
  let basis = $state("");
  let controller: AbortController | undefined;
  const context = $derived<IntakeContext>(intake ? JSON.parse(intake) : {text:"",images:[]});
  const signature = $derived(JSON.stringify({title,intake,body,acceptance,product,project}));
  const stale = $derived(!!result && signature !== basis);
  onDestroy(() => controller?.abort());
  function setText(text: string) { intake = JSON.stringify({...context,text}); }
  const example = "Where: Direct's left sidebar.\nCurrent behavior: I cannot arrange products or group related projects.\nDesired behavior: Drag products to reorder them. Create named sections and move projects into them.\nExample: A Teroka section contains my related sales projects.\nConstraints: Keep the arrangement after restart. Do not change issue status or delete projects.\nScreenshots: Paste the current sidebar here and caption what should change.";
  async function pasteImages(event: ClipboardEvent) {
    const files = Array.from(event.clipboardData?.items || []).filter(item => item.kind === "file" && item.type.startsWith("image/"))
      .map(item => item.getAsFile()).filter((file):file is File => !!file);
    if (!files.length) return;
    event.preventDefault();
    if (disabled || pasting) return;
    if (context.images.length + files.length > 3) { message = "Paste at most three screenshots."; return; }
    pasting = true; message = "";
    try {
      const images = [];
      for (const file of files) {
        if (!["image/png","image/jpeg"].includes(file.type) || file.size > 1024*1024) {
          throw new Error("Paste a PNG or JPEG screenshot up to 1 MB. Crop large screenshots first.");
        }
        const data_url = await new Promise<string>((resolve,reject) => {
          const reader = new FileReader(); reader.onload = () => resolve(String(reader.result));
          reader.onerror = () => reject(new Error("The screenshot could not be read; paste it again."));
          reader.readAsDataURL(file);
        });
        images.push({data_url,caption:""});
      }
      if (context.images.length + images.length > 3) throw new Error("Paste at most three screenshots.");
      intake = JSON.stringify({...context, images:[...context.images,...images]});
    } catch(e) { message = e instanceof Error ? e.message : "Could not paste the screenshot."; }
    finally { pasting = false; }
  }
  function cancel() { controller?.abort(); running = false; message = "Draft canceled. Your input is kept."; }
  async function generate() {
    controller?.abort();
    const active = new AbortController(); controller = active;
    const requested = signature;
    running = true; result = null; message = "";
    try {
      const draft = await draftBrief({title,intake:context,body,acceptance,product,project},active.signal);
      if (active.signal.aborted) return;
      result = draft; basis = requested;
      message = "Review the AI draft below, then apply it. Your original context will be saved with the issue.";
    } catch(e) {
      if (!active.signal.aborted) message = e instanceof Error ? e.message : String(e);
    } finally { if (controller === active) running = false; }
  }
  function apply() {
    if (!result || stale) return;
    body = `Problem\n${result.problem}\n\nExpected outcome\n${result.expected_outcome}`;
    acceptance = result.acceptance.map(item => `- ${item}`).join("\n");
    if (result.questions.length) body += `\n\nOpen questions — resolve before implementation\n${result.questions.map(item => `- ${item}`).join("\n")}`;
    result = null; message = "AI draft applied. Review or edit the brief and criteria before saving.";
  }
</script>

<div class="ai-intake">
  <label class="field">Task context <span class="muted">(optional — all the detail belongs here)</span>
    <textarea rows="5" value={context.text} disabled={disabled} maxlength="40000"
      oninput={event => setText(event.currentTarget.value)} onpaste={pasteImages}
      placeholder="Where is it? What happens now? What should happen? Examples, constraints, and screenshots…"></textarea>
  </label>
  <small>Long titles work too. Paste screenshots with Ctrl+V while focused in task context. PNG/JPEG · up to 3 images · 1 MB each.</small>
  {#each context.images as image, index}
    <figure class="intake-image">
      <img src={image.data_url} alt={image.caption || `Context screenshot ${index+1}`} />
      <label class="field">Screenshot {index+1} caption
        <input value={image.caption} maxlength="2000" disabled={disabled}
          placeholder="What should the agent notice or change?"
          oninput={event => { const images = [...context.images]; images[index] = {...image,caption:event.currentTarget.value}; intake = JSON.stringify({...context,images}); }} />
      </label>
      <button class="text-button" type="button" disabled={disabled}
        onclick={() => { intake = JSON.stringify({...context,images:context.images.filter((_,i)=>i!==index)}); }}>Remove screenshot {index+1}</button>
    </figure>
  {/each}
  <details class="context-example">
    <summary>A useful title and context example</summary>
    <p><b>Title:</b> Reorder sidebar products and group projects into sections</p>
    <p class="prose">{example}</p>
    <button type="button" class="secondary" disabled={disabled || !!context.text.trim()}
      onclick={() => setText(example)}>Use example context</button>
    <small>Describe the outcome in the title. Put observations, desired behavior, examples and boundaries in context. Caption screenshots so the next agent knows why they matter.</small>
  </details>
  <div class="draft-assist">
    <button type="button" class="secondary" disabled={disabled || running || pasting || !title.trim()}
      onclick={generate}>{running ? "Drafting with AI…" : "✦ Draft with AI"}</button>
    {#if running}<button type="button" class="text-button" onclick={cancel}>Cancel AI draft</button>{/if}
    <small>Sends this title, context, screenshots and existing brief to Claude (Opus 5.5) through your signed-in Claude Code. Review before saving.</small>
  </div>
  {#if message}<p class="hint" role="status">{message}</p>{/if}
  {#if result}
    <section class="ai-preview" aria-label="AI draft preview">
      <b>AI draft · Opus 5.5</b>
      <h4>Problem</h4><p class="prose">{result.problem}</p>
      <h4>Expected outcome</h4><p class="prose">{result.expected_outcome}</p>
      <h4>Acceptance criteria</h4><ul>{#each result.acceptance as criterion}<li>{criterion}</li>{/each}</ul>
      {#if result.questions.length}<h4>Open questions</h4><ul>{#each result.questions as question}<li>{question}</li>{/each}</ul>{/if}
      {#if stale}<p role="status">Your inputs changed. Generate a fresh draft before applying it.</p>{/if}
      <button class="secondary" type="button" disabled={disabled || stale} onclick={apply}>
        {body.trim() || acceptance.trim() ? "Replace brief and criteria with AI draft" : "Apply AI draft"}</button>
    </section>
  {/if}
</div>
