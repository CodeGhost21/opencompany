import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

type WorkspaceNode = {
  id: string;
  name: string;
  kind: "file" | "folder";
  parentId?: string | null;
};

async function createNode(
  request: APIRequestContext,
  body: { name: string; kind: "file" | "folder"; parentId?: string; content?: string },
): Promise<WorkspaceNode> {
  const response = await request.post("/api/v1/company/workspace", { data: body });
  expect(response.ok(), `create ${body.name}`).toBeTruthy();
  return (await response.json()) as WorkspaceNode;
}

async function dismissOnboarding(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip.waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
  if (await skip.isVisible()) await skip.click();
}

test("an agent-authored page bundle loads and paints in its sandboxed iframe", async ({
  page,
  request,
}) => {
  page.on("console", (message) => console.log(`[page console] ${message.type()}: ${message.text()}`));
  page.on("requestfailed", (failed) => console.log(`[page request failed] ${failed.url()} ${failed.failure()?.errorText}`));
  page.on("response", (res) => {
    const url = res.url();
    if (url.includes("/pages/") || url.includes("/pages-sdk/")) {
      console.log(`[page response ${res.status()}] ${url}`);
    }
  });
  const slug = `page-e2e-${Date.now().toString(36)}`;
  const title = `E2E page ${slug}`;
  const painted = `Page ${slug} painted`;
  const tree = (await (await request.get("/api/v1/company/workspace")).json()) as WorkspaceNode[];
  let pagesRoot = tree.find((node) => node.kind === "folder" && node.name === "pages");
  const createdPagesRoot = !pagesRoot;
  if (!pagesRoot) pagesRoot = await createNode(request, { name: "pages", kind: "folder" });

  const pageFolder = await createNode(request, {
    name: slug,
    kind: "folder",
    parentId: pagesRoot.id,
  });

  try {
    await createNode(request, {
      name: "page.toml",
      kind: "file",
      parentId: pageFolder.id,
      content: `title = "${title}"\nnav_visible = true\n`,
    });
    await createNode(request, {
      name: "page.compiled.mjs",
      kind: "file",
      parentId: pageFolder.id,
      content: `import { jsx } from "react/jsx-runtime"; export default function Page() { return jsx("h2", { children: "${painted}" }); }`,
    });

    const bundle = page.waitForResponse((response) =>
      response.url().includes(`/pages/${slug}/bundle.mjs`),
    );
    await page.goto("/#/pages");
    await dismissOnboarding(page);

    const bundleResponse = await bundle;
    expect(bundleResponse.status()).toBe(200);
    // TEMP diagnostic: bypass the browser and probe the server directly.
    const shellUrl = `http://127.0.0.1:8689/api/v1/companies/e2e-harness-co/pages/${slug}`;
    const shell = await request.get(shellUrl);
    const shellHtml = await shell.text();
    const capMatch = shellHtml.match(/bootstrap\.mjs\?oc_cap=([a-f0-9]+)/);
    console.log(`[diag shell status] ${shell.status()} hasCap=${!!capMatch}`);
    const bundleUrl = `${shellUrl}/bundle.mjs?oc_cap=${capMatch?.[1] ?? "none"}`;
    const probe = await request.get(bundleUrl, { headers: { Origin: "null" } });
    console.log(`[diag probe] status=${probe.status()}`);
    console.log(`[diag probe headers] ${JSON.stringify(probe.headers())}`);
    console.log(`[diag browser bundle headers] ${JSON.stringify(bundleResponse.headers())}`);
    expect(bundleResponse.headers()["access-control-allow-origin"]).toBe("null");
    expect(bundleResponse.headers()["access-control-allow-credentials"]).toBe("true");
    // TEMP diagnostic: dump the iframe DOM after the module graph settles.
    const frame = page.frames().find((f) => f.url().includes(`/pages/${slug}`));
    console.log(`[diag iframe url] ${frame?.url()}`);
    if (frame) {
      const html = await frame.evaluate(() => document.documentElement.outerHTML);
      console.log(`[diag iframe html len] ${html.length}`);
      console.log(`[diag iframe html] ${html.slice(0, 3000)}`);
    }
    await expect(page.frameLocator("iframe").getByRole("heading", { name: painted })).toBeVisible({
      timeout: 30_000,
    });
  } finally {
    await request.delete(`/api/v1/company/workspace/${pageFolder.id}`);
    if (createdPagesRoot) await request.delete(`/api/v1/company/workspace/${pagesRoot.id}`);
  }
});
