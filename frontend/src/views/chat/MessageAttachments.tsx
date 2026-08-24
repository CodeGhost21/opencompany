import { useEffect, useState } from "react";
import { Download, FileText, Loader2 } from "lucide-react";

import { formatBytes } from "@/api/workspace";
import type { AttachmentDto } from "@/api/types";
import { cn } from "@/lib/utils";

interface Props {
  attachments: AttachmentDto[];
  /**
   * Resolves a stored attachment's bytes to an object URL (issue #1682). The
   * blob route needs the client's bearer, which no `<img>` or bare link can
   * carry, so both the inline preview and the download go through this. Absent
   * when the surface cannot reach the client — the chips then render without a
   * working download rather than crashing.
   */
  resolveUrl?: (nodeId: string) => Promise<string>;
}

/** Whether a stored mime renders as an inline image preview (issue #1682).
 *
 * `image/*` minus SVG: an SVG is an XML document whose `<script>` executes, so
 * the blob route already serves it as an attachment (issue #667) and v1 keeps
 * it download-only rather than inlining it. Every other `image/*` is decoded to
 * pixels with no script context and is safe to preview. */
function isPreviewableImage(mime: string): boolean {
  return mime.startsWith("image/") && mime !== "image/svg+xml";
}

/**
 * The attached files on one message (issue #1682).
 *
 * v1 renders a download chip per file — filename, size, and a click that
 * fetches the bytes through the authenticated blob route and hands them to the
 * browser — plus an inline `<img>` preview for a non-SVG image. Multi-file,
 * drag-drop and rich previews (PDF/video) are deliberately out of scope; the
 * shape already loops so more chips need no change here.
 */
export function MessageAttachments({ attachments, resolveUrl }: Props) {
  if (attachments.length === 0) return null;
  return (
    <div className="mt-1.5 flex flex-col gap-1.5">
      {attachments.map((attachment) => (
        <AttachmentItem key={attachment.nodeId} attachment={attachment} resolveUrl={resolveUrl} />
      ))}
    </div>
  );
}

function AttachmentItem({
  attachment,
  resolveUrl,
}: {
  attachment: AttachmentDto;
  resolveUrl?: (nodeId: string) => Promise<string>;
}) {
  const image = isPreviewableImage(attachment.mime);
  const [previewUrl, setPreviewUrl] = useState<string>();
  const [downloading, setDownloading] = useState(false);

  // Fetch the bytes for an inline image once, and revoke the object URL on
  // unmount so it does not stay resident for the life of the document.
  useEffect(() => {
    if (!image || !resolveUrl) return;
    let url: string | undefined;
    let alive = true;
    void resolveUrl(attachment.nodeId).then((got) => {
      if (alive) {
        url = got;
        setPreviewUrl(got);
      } else {
        URL.revokeObjectURL(got);
      }
    });
    return () => {
      alive = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [image, resolveUrl, attachment.nodeId]);

  async function download() {
    if (!resolveUrl || downloading) return;
    setDownloading(true);
    try {
      const url = await resolveUrl(attachment.nodeId);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = attachment.name;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      // Revoke after the click has been handed off, not synchronously — the
      // browser needs the URL to still resolve when it starts the download.
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } finally {
      setDownloading(false);
    }
  }

  return (
    <div className="flex w-fit max-w-full flex-col gap-1.5">
      {image && previewUrl && (
        <img
          src={previewUrl}
          alt={attachment.name}
          className="max-h-64 max-w-full rounded-md border object-contain"
        />
      )}
      <button
        type="button"
        onClick={() => void download()}
        disabled={!resolveUrl || downloading}
        className={cn(
          "flex w-fit max-w-full items-center gap-2 rounded-md border bg-card px-2.5 py-1.5",
          "text-left text-xs transition-colors hover:bg-accent disabled:opacity-60",
        )}
        title={`Download ${attachment.name}`}
      >
        {downloading ? (
          <Loader2 className="size-4 shrink-0 animate-spin text-muted-foreground" aria-hidden />
        ) : (
          <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        )}
        <span className="min-w-0 truncate font-medium">{attachment.name}</span>
        <span className="shrink-0 text-2xs text-muted-foreground">
          {formatBytes(attachment.size)}
        </span>
        <Download className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      </button>
    </div>
  );
}
