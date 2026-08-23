import * as React from "react";
import * as ReactDOM from "react-dom/client";

const slug = document.documentElement.dataset.pageSlug;
if (!slug) throw new Error("OpenCompany page shell did not provide a slug");

const { default: Page } = await import(new URL(`./${slug}/bundle.mjs`, document.baseURI).href);
const root = ReactDOM.createRoot(document.getElementById("root"));
root.render(React.createElement(Page));
