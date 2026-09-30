#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { chmod, mkdir, rename, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, relative, resolve, sep } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";

const API_URL = "https://api.linear.app/graphql";
const FORMAT_VERSION = 1;
const PAGE_SIZE = 100;
const NESTED_PAGE_SIZE = 250;
const MAX_RETRIES = 4;

const ROOT_TARGETS = [
  "teams",
  "workflowStates",
  "issueLabels",
  "projects",
  "projectMilestones",
  "initiatives",
  "projectUpdates",
  "initiativeUpdates",
  "issues",
  "comments",
  "documents",
  "users",
  "issueRelations",
  "attachments",
];

const REQUIRED_TARGETS = new Set([
  "teams",
  "projects",
  "initiatives",
  "issues",
  "comments",
  "documents",
]);

const REFERENCE_FIELDS = new Set([
  "assignee",
  "creator",
  "cycle",
  "document",
  "initiative",
  "issue",
  "lead",
  "organization",
  "owner",
  "parent",
  "project",
  "projectMilestone",
  "relatedIssue",
  "sourceIssue",
  "state",
  "status",
  "targetIssue",
  "team",
  "user",
  "workspace",
]);

const NESTED_CONNECTIONS = {
  // Parent links, issue relations, and attachments are captured through their
  // own root collections. Labels and history need issue-scoped connections.
  Issue: new Set(["history", "labels"]),
  Project: new Set(["initiatives", "milestones", "projectMilestones", "teams"]),
  Initiative: new Set(["projects", "teams"]),
};

class RetryableError extends Error {}

const TYPE_INTROSPECTION_QUERY = String.raw`
  query DirectLinearSourceType($name: String!) {
    __type(name: $name) {
      kind
      name
      fields(includeDeprecated: true) {
        name
        isDeprecated
        deprecationReason
        args {
          name
          defaultValue
          type {
            kind name
            ofType {
              kind name
              ofType {
                kind name
                ofType { kind name }
              }
            }
          }
        }
        type {
          kind name
          ofType {
            kind name
            ofType {
              kind name
              ofType { kind name }
            }
          }
        }
      }
    }
  }
`;

if (process.argv.includes("--self-test")) {
  runSelfTest();
  process.exit(0);
}

const apiKey = process.env.LINEAR_API_KEY?.trim();
if (!apiKey) {
  fail(
    "LINEAR_API_KEY is required. Use a temporary read-only key; do not save it in the repository.",
  );
}

const capturedAt = new Date().toISOString();
const stamp = capturedAt.replaceAll(":", "-").replace(".", "-");
const requestedOutput =
  argumentValue("--out") ??
  resolve(homedir(), ".direct", "linear-migrations", `linear-source-${stamp}`);
const outputDirectory = resolve(requestedOutput);
const repositoryRoot = resolve(process.cwd());

if (isInside(outputDirectory, repositoryRoot)) {
  fail(
    `Refusing to place private Linear data inside the repository: ${outputDirectory}`,
  );
}

const dataDirectory = resolve(outputDirectory, "data");
const attachmentDirectory = resolve(outputDirectory, "attachments");
const errors = [];
const missingCoverage = [];
const queryCoverage = [];

await mkdir(dataDirectory, { recursive: true, mode: 0o700 });
await mkdir(attachmentDirectory, { recursive: true, mode: 0o700 });
await chmod(outputDirectory, 0o700).catch(() => {});

console.log(`Capturing Linear source data into ${outputDirectory}`);

const types = new Map();
const queryType = await loadType("Query");
if (!queryType) fail("Linear GraphQL schema did not expose its query type.");

const records = new Map();
for (const rootName of ROOT_TARGETS) {
  const field = fieldNamed(queryType, rootName);
  if (!field) {
    if (REQUIRED_TARGETS.has(rootName)) {
      missingCoverage.push(
        `${rootName}: root query is not exposed by the current Linear API schema`,
      );
    }
    continue;
  }

  try {
    const captured = await captureRootConnection(field);
    records.set(rootName, captured.nodes);
    queryCoverage.push(captured.coverage);
    await writePrivateJson(resolve(dataDirectory, `${kebab(rootName)}.json`), {
      captured_at: capturedAt,
      count: captured.nodes.length,
      records: sanitizeSignedUrls(captured.nodes),
    });
    console.log(`${rootName}: ${captured.nodes.length}`);
  } catch (error) {
    recordError(`${rootName} capture`, error);
    if (REQUIRED_TARGETS.has(rootName)) {
      missingCoverage.push(`${rootName}: capture failed; see errors`);
    }
  }
}

const capturedSchema = {
  query_type: "Query",
  types: [...types.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  ),
};
const schemaJson = stableJson(capturedSchema);
const schemaSha256 = sha256(schemaJson);
await writePrivateJson(
  resolve(dataDirectory, "graphql-schema.json"),
  capturedSchema,
);

const issueRecords = records.get("issues") ?? [];
const teamRecords = records.get("teams") ?? [];
const perTeam = buildTeamCounts(teamRecords, issueRecords);
const allRecords = [...records.entries()].map(([name, nodes]) => ({
  name,
  nodes,
}));
const discoveredAssets = discoverUploadUrls(allRecords);
const attachmentResults = await downloadAssets(discoveredAssets);
const attachmentManifest = attachmentResults.map((item) =>
  sanitizeSignedUrls(item),
);
await writePrivateJson(
  resolve(outputDirectory, "attachment-manifest.json"),
  attachmentManifest,
);

const inaccessibleAttachments = attachmentResults.filter(
  (item) => item.status !== "downloaded",
);
if (inaccessibleAttachments.length > 0) {
  missingCoverage.push(
    `${inaccessibleAttachments.length} uploaded attachment(s) could not be downloaded or checksummed; see attachment-manifest.json`,
  );
}

for (const coverage of queryCoverage) {
  if (
    !coverage.include_archived_supported &&
    REQUIRED_TARGETS.has(coverage.root)
  ) {
    missingCoverage.push(
      `${coverage.root}: the current root query has no includeArchived argument, so archived coverage cannot be proven`,
    );
  }
  for (const nested of coverage.truncated_nested_connections) {
    missingCoverage.push(
      `${coverage.root}: nested connection ${nested} exceeded ${NESTED_PAGE_SIZE} records`,
    );
  }
}

missingCoverage.push(
  "Deleted records that are no longer returned by Linear are not recoverable through the public API.",
  "External link attachments are preserved as metadata but their third-party contents are not downloaded.",
);

const manifest = {
  format: FORMAT_VERSION,
  captured_at: capturedAt,
  source: {
    service: "Linear",
    api_url: API_URL,
    workspace_slug: "canvasm",
    mode: "read-only GraphQL queries and authenticated attachment downloads",
  },
  output_directory: outputDirectory,
  schema_sha256: schemaSha256,
  counts: Object.fromEntries(
    [...records].map(([name, nodes]) => [name, nodes.length]),
  ),
  per_team_issue_counts: perTeam,
  attachments: {
    discovered_upload_urls: discoveredAssets.length,
    downloaded_and_checksummed:
      attachmentResults.length - inaccessibleAttachments.length,
    inaccessible_or_failed: inaccessibleAttachments.length,
    link_attachment_records: (records.get("attachments") ?? []).length,
  },
  query_coverage: queryCoverage,
  errors,
  missing_coverage: [...new Set(missingCoverage)],
  integrity: {
    manifest_sha256: "SELF (see manifest.sha256)",
    data_files: await fileChecksums(dataDirectory),
    attachment_files: attachmentResults
      .filter((item) => item.status === "downloaded")
      .map((item) => ({
        path: item.path,
        sha256: item.sha256,
        bytes: item.bytes,
      })),
  },
};

const manifestPath = resolve(outputDirectory, "manifest.json");
await writePrivateJson(manifestPath, manifest);
const manifestBytes = await import("node:fs/promises").then(({ readFile }) =>
  readFile(manifestPath),
);
await writeFile(
  resolve(outputDirectory, "manifest.sha256"),
  `${sha256(manifestBytes)}  manifest.json\n`,
  { mode: 0o600, flag: "wx" },
);

console.log(`Manifest: ${manifestPath}`);
console.log(`Issues captured: ${issueRecords.length}`);
console.log(
  `Attachments checksummed: ${manifest.attachments.downloaded_and_checksummed}`,
);
if (errors.length || inaccessibleAttachments.length) process.exitCode = 2;

async function captureRootConnection(rootField) {
  const connectionTypeName = namedType(rootField.type).name;
  const connectionType = await loadType(connectionTypeName);
  const nodesField = connectionType && fieldNamed(connectionType, "nodes");
  if (!nodesField)
    throw new Error(`${rootField.name} does not return a nodes connection`);
  const nodeTypeName = namedType(nodesField.type).name;
  await hydrateSelectionTypes(nodeTypeName, true);
  const acceptedArgs = new Map(rootField.args.map((arg) => [arg.name, arg]));
  const truncated = new Set();
  const base = await capturePages(
    nodeSelection(nodeTypeName, false),
    PAGE_SIZE,
    "base",
  );
  const nodes = base.nodes;
  let pages = base.pages;
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const failedNestedConnections = [];
  const nestedFields = NESTED_CONNECTIONS[nodeTypeName] ?? new Set();

  for (const fieldName of nestedFields) {
    const nestedSelection = nestedConnectionSelection(nodeTypeName, fieldName);
    if (!nestedSelection) continue;
    const identifyingFields = scalarFields(nodeTypeName).includes("identifier")
      ? "id identifier"
      : "id";
    try {
      const nested = await capturePages(
        `${identifyingFields}\n${nestedSelection}`,
        10,
        fieldName,
      );
      pages += nested.pages;
      for (const partialNode of nested.nodes) {
        const target = byId.get(partialNode.id);
        if (target && Object.hasOwn(partialNode, fieldName)) {
          target[fieldName] = partialNode[fieldName];
        }
      }
      findTruncatedNestedConnections(nested.nodes, rootField.name, truncated);
    } catch (error) {
      failedNestedConnections.push(fieldName);
      recordError(`${rootField.name}.${fieldName} capture`, error);
      missingCoverage.push(
        `${rootField.name}: nested connection ${fieldName} failed; see errors`,
      );
    }
  }

  return {
    nodes,
    coverage: {
      root: rootField.name,
      node_type: nodeTypeName,
      records: nodes.length,
      pages,
      capture_mode: nestedFields.size
        ? "base plus split nested connections"
        : "base",
      include_archived_supported: acceptedArgs.has("includeArchived"),
      failed_nested_connections: failedNestedConnections,
      truncated_nested_connections: [...truncated].sort(),
    },
  };

  async function capturePages(selection, pageSize, label) {
    const variableDefinitions = [];
    const callArgs = [];
    const baseVariables = {};

    for (const [name, value] of [
      ["first", pageSize],
      ["after", null],
      ["includeArchived", true],
    ]) {
      const argument = acceptedArgs.get(name);
      if (!argument) continue;
      variableDefinitions.push(`$${name}: ${typeString(argument.type)}`);
      callArgs.push(`${name}: $${name}`);
      baseVariables[name] = value;
    }

    const definitions = variableDefinitions.length
      ? `(${variableDefinitions.join(", ")})`
      : "";
    const document = `query DirectCapture${definitions} {
      capture: ${rootField.name}${callArgs.length ? `(${callArgs.join(", ")})` : ""} {
        nodes { ${selection} }
        pageInfo { hasNextPage endCursor }
      }
    }`;

    const captured = [];
    let after = null;
    let pageCount = 0;
    do {
      const variables = { ...baseVariables };
      if (Object.hasOwn(variables, "after")) variables.after = after;
      const data = await graphql(
        document,
        variables,
        `${rootField.name} ${label} page ${pageCount + 1}`,
      );
      const page = data.capture;
      if (!page) throw new Error(`${rootField.name} returned no connection`);
      captured.push(...page.nodes);
      pageCount += 1;
      if (page.pageInfo?.hasNextPage && !acceptedArgs.has("after")) {
        throw new Error(
          `${rootField.name} cannot paginate because it has no after argument`,
        );
      }
      after = page.pageInfo?.hasNextPage ? page.pageInfo.endCursor : null;
    } while (after);

    return { nodes: captured, pages: pageCount };
  }
}

async function loadType(name) {
  if (!name) return null;
  if (types.has(name)) return types.get(name);
  const data = await graphql(
    TYPE_INTROSPECTION_QUERY,
    { name },
    `schema type ${name}`,
  );
  if (!data.__type) return null;
  types.set(name, data.__type);
  return data.__type;
}

async function hydrateSelectionTypes(
  typeName,
  includeNested,
  seen = new Set(),
) {
  const marker = `${typeName}:${includeNested}`;
  if (seen.has(marker)) return;
  seen.add(marker);

  const type = await loadType(typeName);
  if (!type?.fields) return;

  for (const field of type.fields) {
    if (hasRequiredArguments(field)) continue;
    const target = namedType(field.type);
    if (target.kind === "OBJECT" && REFERENCE_FIELDS.has(field.name)) {
      await loadType(target.name);
    }
  }

  if (!includeNested) return;
  const allowed = NESTED_CONNECTIONS[typeName] ?? new Set();
  for (const fieldName of allowed) {
    const field = fieldNamed(type, fieldName);
    if (!field) continue;
    const connectionType = await loadType(namedType(field.type).name);
    const nodesField = connectionType && fieldNamed(connectionType, "nodes");
    if (!nodesField) continue;
    await hydrateSelectionTypes(namedType(nodesField.type).name, false, seen);
  }
}

function nodeSelection(typeName, includeNested) {
  const type = types.get(typeName);
  if (!type?.fields) throw new Error(`Unknown GraphQL object type ${typeName}`);
  const selections = [];

  for (const field of type.fields) {
    if (hasRequiredArguments(field)) continue;
    const target = namedType(field.type);
    if (target.kind === "SCALAR" || target.kind === "ENUM") {
      selections.push(field.name);
      continue;
    }
    if (target.kind === "OBJECT" && REFERENCE_FIELDS.has(field.name)) {
      const reference = scalarFields(target.name);
      if (reference.length)
        selections.push(`${field.name} { ${reference.join(" ")} }`);
    }
  }

  if (includeNested) {
    const allowed = NESTED_CONNECTIONS[typeName] ?? new Set();
    for (const fieldName of allowed) {
      const nested = nestedConnectionSelection(typeName, fieldName);
      if (nested) selections.push(nested);
    }
  }

  return selections.sort().join("\n");
}

function nestedConnectionSelection(typeName, fieldName) {
  const type = types.get(typeName);
  const field = fieldNamed(type, fieldName);
  if (!field) return null;
  const connectionType = types.get(namedType(field.type).name);
  const nodesField = connectionType && fieldNamed(connectionType, "nodes");
  if (!nodesField) return null;
  const childTypeName = namedType(nodesField.type).name;
  const childSelection =
    typeName === "Issue" && fieldName === "history"
      ? nodeSelection(childTypeName, false)
      : relationshipSelection(childTypeName);
  const args = [];
  if (field.args.some((argument) => argument.name === "first")) {
    args.push(`first: ${NESTED_PAGE_SIZE}`);
  }
  if (field.args.some((argument) => argument.name === "includeArchived")) {
    args.push("includeArchived: true");
  }
  return `${fieldName}${args.length ? `(${args.join(", ")})` : ""} { nodes { ${childSelection} } pageInfo { hasNextPage endCursor } }`;
}

function relationshipSelection(typeName) {
  const available = new Set(scalarFields(typeName));
  const preferred = [
    "id",
    "identifier",
    "slugId",
    "key",
    "name",
    "title",
    "type",
    "url",
  ];
  const selected = preferred.filter((field) => available.has(field));
  if (!selected.includes("id") && available.has("id")) selected.unshift("id");
  if (!selected.length) {
    throw new Error(
      `No scalar relationship identifier is available on ${typeName}`,
    );
  }
  return selected.join(" ");
}

function scalarFields(typeName) {
  const type = types.get(typeName);
  if (!type?.fields) return [];
  return type.fields
    .filter((field) => !hasRequiredArguments(field))
    .filter((field) => ["SCALAR", "ENUM"].includes(namedType(field.type).kind))
    .map((field) => field.name)
    .sort();
}

async function graphql(query, variables, label) {
  let lastError;
  for (let attempt = 0; attempt < MAX_RETRIES; attempt += 1) {
    try {
      const response = await fetch(API_URL, {
        method: "POST",
        headers: {
          Accept: "application/json",
          Authorization: apiKey,
          "Content-Type": "application/json",
          "public-file-urls-expire-in": "3600",
        },
        body: JSON.stringify({ query, variables }),
      });
      const payload = await response.json().catch(() => null);
      if (response.status === 429 || response.status >= 500) {
        throw new RetryableError(
          `${label}: Linear returned HTTP ${response.status}`,
        );
      }
      if (payload?.errors?.length) {
        throw new Error(
          `${label}: ${payload.errors.map((item) => item.message).join("; ")}`,
        );
      }
      if (!response.ok)
        throw new Error(`${label}: Linear returned HTTP ${response.status}`);
      if (!payload?.data)
        throw new Error(`${label}: response contained no data`);
      return payload.data;
    } catch (error) {
      lastError = error;
      if (!(error instanceof RetryableError) || attempt === MAX_RETRIES - 1)
        break;
      await new Promise((resolveWait) =>
        setTimeout(resolveWait, 500 * 2 ** attempt),
      );
    }
  }
  throw lastError;
}

function buildTeamCounts(teams, issues) {
  const counts = new Map();
  for (const team of teams) {
    counts.set(team.id, {
      id: team.id,
      key: team.key ?? null,
      name: team.name ?? null,
      url: team.url ?? null,
      active: 0,
      archived: 0,
      total: 0,
    });
  }
  for (const issue of issues) {
    const teamId = issue.team?.id ?? "unknown";
    const count = counts.get(teamId) ?? {
      id: teamId,
      key: issue.team?.key ?? null,
      name: issue.team?.name ?? "Unknown team",
      url: issue.team?.url ?? null,
      active: 0,
      archived: 0,
      total: 0,
    };
    count.total += 1;
    if (issue.archivedAt) count.archived += 1;
    else count.active += 1;
    counts.set(teamId, count);
  }
  return [...counts.values()].sort((left, right) =>
    (left.name ?? "").localeCompare(right.name ?? ""),
  );
}

function discoverUploadUrls(groups) {
  const assets = new Map();
  for (const group of groups) {
    walkStrings(group.nodes, [group.name], (value, path) => {
      for (const url of uploadUrls(value)) {
        const canonicalUrl = canonicalUploadUrl(url);
        const existing = assets.get(canonicalUrl) ?? {
          canonical_url: canonicalUrl,
          signed_url: url,
          source_paths: [],
        };
        if (existing.source_paths.length < 50)
          existing.source_paths.push(path.join("."));
        if (url.includes("?")) existing.signed_url = url;
        assets.set(canonicalUrl, existing);
      }
    });
  }
  return [...assets.values()].sort((left, right) =>
    left.canonical_url.localeCompare(right.canonical_url),
  );
}

async function downloadAssets(assets) {
  const results = [];
  for (let index = 0; index < assets.length; index += 1) {
    const asset = assets[index];
    const result = {
      canonical_url: asset.canonical_url,
      source_paths: asset.source_paths,
      status: "failed",
    };
    const temporaryPath = resolve(
      attachmentDirectory,
      `.partial-${randomUUID()}`,
    );
    try {
      const response = await fetchAsset(asset.signed_url);
      if (!response.body) throw new Error("attachment response had no body");
      const contentDisposition = response.headers.get("content-disposition");
      const originalName = attachmentName(
        asset.canonical_url,
        contentDisposition,
      );
      const hash = createHash("sha256");
      let bytes = 0;
      const checksumStream = new Transform({
        transform(chunk, _encoding, callback) {
          hash.update(chunk);
          bytes += chunk.length;
          callback(null, chunk);
        },
      });
      await pipeline(
        Readable.fromWeb(response.body),
        checksumStream,
        createWriteStream(temporaryPath, { flags: "wx", mode: 0o600 }),
      );
      const digest = hash.digest("hex");
      const storedName = `${digest}-${safeFilename(originalName)}`;
      const finalPath = resolve(attachmentDirectory, storedName);
      await rename(temporaryPath, finalPath).catch(async (error) => {
        if (error.code !== "EEXIST") throw error;
        await rm(temporaryPath, { force: true });
      });
      Object.assign(result, {
        status: "downloaded",
        path: relative(outputDirectory, finalPath).replaceAll("\\", "/"),
        original_name: originalName,
        content_type: response.headers.get("content-type"),
        bytes,
        sha256: digest,
      });
      console.log(`attachment ${index + 1}/${assets.length}: ${storedName}`);
    } catch (error) {
      await rm(temporaryPath, { force: true }).catch(() => {});
      result.error = errorMessage(error);
      recordError(`attachment ${asset.canonical_url}`, error);
    }
    results.push(result);
  }
  return results;
}

async function fetchAsset(url) {
  const attempts = [
    {},
    { headers: { Authorization: apiKey } },
    { headers: { Authorization: `Bearer ${apiKey}` } },
  ];
  let lastStatus = 0;
  for (const options of attempts) {
    const response = await fetch(url, { ...options, redirect: "follow" });
    lastStatus = response.status;
    if (response.ok) return response;
    if (![401, 403].includes(response.status)) break;
  }
  throw new Error(`attachment download returned HTTP ${lastStatus}`);
}

function findTruncatedNestedConnections(nodes, rootName, found) {
  for (const node of nodes) {
    for (const [key, value] of Object.entries(node ?? {})) {
      if (value?.pageInfo?.hasNextPage) {
        found.add(
          `${rootName}.${node.identifier ?? node.id ?? "unknown"}.${key}`,
        );
      }
    }
  }
}

function walkStrings(value, path, visit) {
  if (typeof value === "string") {
    visit(value, path);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) =>
      walkStrings(item, [...path, String(index)], visit),
    );
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value))
      walkStrings(item, [...path, key], visit);
  }
}

function uploadUrls(value) {
  const matches =
    value.match(/https:\/\/uploads\.linear\.app\/[^\s<>"']+/giu) ?? [];
  return matches.map((url) => url.replace(/[),.;!?\]}]+$/u, ""));
}

function canonicalUploadUrl(value) {
  const url = new URL(value);
  url.search = "";
  url.hash = "";
  return url.toString();
}

function sanitizeSignedUrls(value) {
  if (typeof value === "string") {
    return value.replace(
      /https:\/\/uploads\.linear\.app\/[^\s<>"']+/giu,
      (url) => {
        const trailing = url.match(/[),.;!?\]}]+$/u)?.[0] ?? "";
        const bare = trailing ? url.slice(0, -trailing.length) : url;
        try {
          return `${canonicalUploadUrl(bare)}${trailing}`;
        } catch {
          return url;
        }
      },
    );
  }
  if (Array.isArray(value)) return value.map(sanitizeSignedUrls);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        sanitizeSignedUrls(item),
      ]),
    );
  }
  return value;
}

function attachmentName(url, contentDisposition) {
  const utf8 = contentDisposition?.match(/filename\*=UTF-8''([^;]+)/iu)?.[1];
  const basic = contentDisposition?.match(/filename="?([^";]+)"?/iu)?.[1];
  const candidate = utf8 ? decodeURIComponent(utf8) : basic;
  if (candidate) return candidate;
  try {
    return (
      decodeURIComponent(basename(new URL(url).pathname)) || "attachment.bin"
    );
  } catch {
    return "attachment.bin";
  }
}

function safeFilename(value) {
  const safe = value.replace(/[<>:"/\\|?*\u0000-\u001f]/gu, "_").trim();
  return (safe || "attachment.bin").slice(-180);
}

async function writePrivateJson(path, value) {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  await writeFile(path, `${stableJson(value)}\n`, { mode: 0o600, flag: "wx" });
}

async function fileChecksums(directory) {
  const { readdir, readFile } = await import("node:fs/promises");
  const entries = await readdir(directory, { withFileTypes: true });
  const checksums = [];
  for (const entry of entries
    .filter((item) => item.isFile())
    .sort((a, b) => a.name.localeCompare(b.name))) {
    const path = resolve(directory, entry.name);
    const bytes = await readFile(path);
    checksums.push({
      path: relative(outputDirectory, path).replaceAll("\\", "/"),
      bytes: bytes.length,
      sha256: sha256(bytes),
    });
  }
  return checksums;
}

function fieldNamed(type, name) {
  return type?.fields?.find((field) => field.name === name);
}

function hasRequiredArguments(field) {
  return field.args.some(
    (argument) =>
      argument.type.kind === "NON_NULL" && argument.defaultValue == null,
  );
}

function namedType(type) {
  let current = type;
  while (current?.ofType) current = current.ofType;
  return current ?? {};
}

function typeString(type) {
  if (type.kind === "NON_NULL") return `${typeString(type.ofType)}!`;
  if (type.kind === "LIST") return `[${typeString(type.ofType)}]`;
  return type.name;
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  if (index < 0) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) fail(`${name} requires a value`);
  return value;
}

function isInside(candidate, parent) {
  const rel = relative(parent, candidate);
  return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== "..");
}

function stableJson(value) {
  return JSON.stringify(value, null, 2);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function kebab(value) {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

function recordError(scope, error) {
  errors.push({ scope, message: errorMessage(error) });
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function runSelfTest() {
  const testRepositoryRoot = resolve(process.cwd());
  const signed =
    "https://uploads.linear.app/path/example.png?signature=secret&expires=1";
  const sample = `![image](${signed}) and ${signed}.`;
  const found = uploadUrls(sample);
  if (found.length !== 2)
    throw new Error(`expected two upload URL references, got ${found.length}`);
  if (
    canonicalUploadUrl(found[0]) !==
    "https://uploads.linear.app/path/example.png"
  ) {
    throw new Error("canonical upload URL retained its signature");
  }
  const sanitized = sanitizeSignedUrls({ body: sample });
  if (sanitized.body.includes("signature="))
    throw new Error("signed URL was not sanitized");
  if (!isInside(resolve(testRepositoryRoot, "private"), testRepositoryRoot)) {
    throw new Error("repository containment check rejected a child path");
  }
  if (
    isInside(resolve(testRepositoryRoot, "..", "private"), testRepositoryRoot)
  ) {
    throw new Error("repository containment check accepted a sibling path");
  }
  if (safeFilename('a<b>:c"d/e\\f|g?h*.png') !== "a_b__c_d_e_f_g_h_.png") {
    throw new Error("unsafe filename characters were not replaced");
  }
  console.log("capture-linear self-test passed");
}
