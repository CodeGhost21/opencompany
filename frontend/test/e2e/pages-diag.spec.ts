import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

// TEMPORARY diagnostic spec — delete before push.

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

test("diag: capture iframe module graph", async ({ page, request }) => {
  page.on("console", (message) => {
    console.log(`[console ${message.type()}] ${message.text()}`);
  });
  page.on("requestfailed", (failed) =>
    console.log(`[reqfailed] ${failed.url()} ${failed.failure()?.errorText}`),
  );
  page.on("request", (req) => {
    const url = req.url();
    if (url.includes("/pages/") || url.includes("/pages-sdk/")) {
      console.log(`[request] ${req.method()} ${url}`);
    }
  });
  page.on("response", (res) => {
    const url = res.url();
    if (url.includes("/pages/") || url.includes("/pages-sdk/")) {
      console.log(`[response ${res.status()}] ${url}`);
    }
  });

  const slug = `diag-${Date.now().toString(36)}`;
  const title = `Diag ${slug}`;
  const painted = `Diag ${slug} painted`;
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

    await page.goto("/#/pages");
    const frame = page.frames().find((f) => f.url().includes(`/pages/${slug}`));
    expect(frame, "shell iframe frame should exist").toBeTruthy();

    // Give the module graph time to settle, then dump what the iframe DOM looks like.
    await page.waitForTimeout(3000);
    console.log(`[iframe url] ${frame!.url()}`);
    const html = await frame!.evaluate(() => document.documentElement.outerHTML);
    console.log(`[iframe html head] ${html.slice(0, 2000)}`);
    console.log(`[iframe has root child] ${html.includes("Diag")}`);
    console.log(`[iframe root children] ${html.match(/<div id="root">([\s\S]*?)<\/div>/)?.[1]?.slice(0, 500) ?? "no root div"}`);
  } finally {
    await request.delete(`/api/v1/company/workspace/${pageFolder.id}`).catch(() => {});
    if (createdPagesRoot) await request.delete(`/api/v1/company/workspace/${pagesRoot.id}`).catch(() => {});
  }
});
