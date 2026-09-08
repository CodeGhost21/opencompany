import { useEffect, useState } from "react";
import { Loader2, TriangleAlert } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { sendInvoice, type Invoice } from "@/api/finance";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useLocalScope } from "@/connections/ConnectionContext";
import {
  clearUnresolvedForceNew,
  readUnresolvedForceNew,
  writeUnresolvedForceNew,
} from "@/views/finance/forceNewNonceStore";
import { deriveInvoiceIdempotencyKey } from "@/views/finance/invoiceKey";
import { fromMinorUnits, toMinorUnits } from "@/views/finance/money";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The Chargebee site, so the confirm line can name which one is billed. */
  site: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSent: (invoice: Invoice) => void;
}

/**
 * A Chargebee site whose slug does not end in `-test` is presumed live.
 *
 * A heuristic, and named as one. Chargebee's API does not report whether a site
 * is a test site, and the alternative — saying nothing — means the loudest
 * warning on the page is absent exactly when it matters. Wrong in the safe
 * direction: a test site named without the suffix gets a warning it did not
 * need, which costs a moment's reading.
 */
function looksLive(site: string | null): boolean {
  return !!site && !/-test$/i.test(site.trim());
}

/**
 * The due-days field, parsed once.
 *
 * Returns `undefined` for an empty field (Chargebee's site default) and the
 * integer for a valid input; `null` marks malformed input — not a number, a
 * decimal, a negative, or an integer too large for the wire — which must not
 * silently become "no due date" or a rejected body.
 */
function parseDueDays(raw: string): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return undefined;
  const value = Number(trimmed);
  if (!Number.isInteger(value) || value < 0 || !Number.isSafeInteger(value)) return null;
  return value;
}

/**
 * Raise an invoice against a real customer.
 *
 * The only destructive thing in the Finance section, and there is no route that
 * undoes it. Three things this dialog has to get right:
 *
 * # Minor units
 *
 * The form takes dollars-and-cents and converts with `toMinorUnits`, which is
 * string arithmetic — never `amount * 100`, which drifts. The minor-unit value
 * that will actually be sent is rendered under the field, because the host's
 * `*_in_minor_units` naming exists precisely so "invoice Alan $100" cannot
 * become a $1.00 invoice, and a UI that hid the unit would re-open that hole
 * from the other side.
 *
 * # Idempotency
 *
 * The key is derived from the invoice's own content — see
 * `deriveInvoiceIdempotencyKey` — not from when the dialog opened. A
 * double-clicked Send is one invoice, and so is a failed send followed by a
 * close-reopen-resend of the *same* invoice, because the fields that identify
 * it hash the same either way. Chargebee replays the original for a repeated
 * key and the reply is byte-identical to a fresh one, so
 * `replayed_earlier_invoice` is the only way to tell — and the toast says
 * "already sent" rather than "sent" when it is set. `invoice-force-new`
 * mints a nonce into the hash for the rare deliberate duplicate — once, when
 * the box is checked, not per send, and it survives a retry of that same
 * forced send, close-reopen included, until it succeeds or the operator
 * unchecks the box to start a separate deliberate resend. An
 * attempted-and-unresolved nonce is latched (`forceNewNonceStore`) against
 * the invoice it was minted for, so retyping that same invoice after a
 * reload re-adopts it while a different invoice gets its own. Otherwise a
 * second ambiguous timeout on the forced path would mint a second real
 * invoice — the exact failure this dialog exists to prevent. The box is
 * frozen while a send is in flight, so a mid-flight toggle cannot replace
 * the nonce the pending request already carries.
 *
 * # Naming the money
 *
 * The confirm line states the recipient, the amount in the currency, and the
 * site, flagging a live one. Not "Are you sure?", which asks a question the
 * operator cannot answer without the facts it omits.
 */
export function SendInvoiceDialog({
  client,
  company,
  site,
  open,
  onOpenChange,
  onSent,
}: Props) {
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [currency, setCurrency] = useState("USD");
  const [description, setDescription] = useState("");
  const [amount, setAmount] = useState("");
  const [dueDays, setDueDays] = useState("");
  const [busy, setBusy] = useState(false);
  const scope = useLocalScope();
  const [forceNewNonce, setForceNewNonce] = useState<string>();
  const [forceNew, setForceNew] = useState(false);
  const [forceNewAttempted, setForceNewAttempted] = useState(false);

  // Reset on open — but only when the last checked box never reached an
  // attempted send. An attempted-and-unresolved forced send (ambiguous
  // failure) must survive a close/reopen of the same invoice so a retry
  // reuses its nonce instead of raising a second real one; only a box that
  // was checked and abandoned (cancelled, never sent) resets.
  useEffect(() => {
    if (open && !forceNewAttempted) {
      setForceNew(false);
      setForceNewNonce(undefined);
    }
  }, [open, forceNewAttempted]);

  const minor = toMinorUnits(amount, currency);
  const live = looksLive(site);
  const due = parseDueDays(dueDays);
  const ready =
    email.trim() !== "" &&
    description.trim() !== "" &&
    minor !== null &&
    minor > 0 &&
    due !== null;

  // The invoice's own identity, independent of any nonce. This is what a
  // held nonce is matched against, so a nonce minted for one invoice can
  // never be folded into another's key.
  const invoiceKey =
    minor !== null && due !== null
      ? deriveInvoiceIdempotencyKey({
          customerEmail: email,
          currencyCode: currency,
          dueDays: due,
          lineItems: [{ description, amountInMinorUnits: minor }],
        })
      : undefined;

  // Re-adopt an unresolved forced send only once the operator has retyped
  // the same invoice it was minted for. A remount restores nothing on its
  // own: the form is blank, and a blank form is not that invoice.
  useEffect(() => {
    if (!open || !invoiceKey) return;
    const held = readUnresolvedForceNew(scope);
    if (held && held.invoiceKey === invoiceKey && forceNewNonce !== held.nonce) {
      setForceNewNonce(held.nonce);
      setForceNew(true);
      setForceNewAttempted(true);
    }
  }, [open, invoiceKey, scope, forceNewNonce]);

  async function onSubmit() {
    if (minor === null || minor <= 0 || due === null) return;
    setBusy(true);
    if (forceNew) {
      setForceNewAttempted(true);
      if (forceNewNonce && invoiceKey) {
        writeUnresolvedForceNew(scope, { invoiceKey, nonce: forceNewNonce });
      }
    }
    try {
      const idempotencyKey = deriveInvoiceIdempotencyKey(
        {
          customerEmail: email,
          currencyCode: currency,
          dueDays: due,
          lineItems: [{ description, amountInMinorUnits: minor }],
        },
        forceNew ? forceNewNonce : undefined,
      );
      const invoice = await sendInvoice(client, company, {
        customer_email: email.trim(),
        customer_name: name.trim() || undefined,
        currency_code: currency.trim().toUpperCase(),
        line_items: [{ description: description.trim(), amount_in_minor_units: minor }],
        due_days: due,
        idempotency_key: idempotencyKey,
      });
      if (invoice.replayed_earlier_invoice) {
        toast.info(
          `Already sent — Chargebee returned the existing invoice ${invoice.id} rather than billing again.`,
        );
      } else {
        toast.success(`Invoice ${invoice.id} sent to ${email.trim()}.`);
      }
      onSent(invoice);
      onOpenChange(false);
      setEmail("");
      setName("");
      setDescription("");
      setAmount("");
      setDueDays("");
      setForceNew(false);
      setForceNewNonce(undefined);
      setForceNewAttempted(false);
      clearUnresolvedForceNew(scope);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not raise the invoice.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg" data-testid="send-invoice-dialog">
        <DialogHeader>
          <DialogTitle>Send an invoice</DialogTitle>
          <DialogDescription>
            Creates the customer in Chargebee if they do not exist yet, then raises and posts the
            invoice.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="inv-email">Customer email</Label>
              <Input
                id="inv-email"
                data-testid="invoice-email"
                type="email"
                placeholder="alan@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="inv-name">Name (new customers only)</Label>
              <Input
                id="inv-name"
                data-testid="invoice-name"
                placeholder="Alan Turing"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="inv-desc">Line item</Label>
            <Input
              id="inv-desc"
              data-testid="invoice-description"
              placeholder="Consulting, March"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>

          <div className="grid gap-4 sm:grid-cols-3">
            <div className="space-y-2">
              <Label htmlFor="inv-currency">Currency</Label>
              <Input
                id="inv-currency"
                data-testid="invoice-currency"
                value={currency}
                onChange={(e) => setCurrency(e.target.value.toUpperCase())}
              />
            </div>
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor="inv-amount">Amount</Label>
              <Input
                id="inv-amount"
                data-testid="invoice-amount"
                inputMode="decimal"
                placeholder="1250.00"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
              />
              {/* The unit, stated. This is the whole guard against a $100
                  invoice arriving as $1.00 or $10,000.00. */}
              <p className="text-xs text-muted-foreground" data-testid="invoice-minor-units">
                {amount.trim() === ""
                  ? `Sent to Chargebee in ${currency} minor units.`
                  : minor === null
                    ? `Not a valid ${currency} amount.`
                    : `Sends ${minor} minor units — ${fromMinorUnits(minor, currency)}.`}
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="inv-due">Due in (days)</Label>
            <Input
              id="inv-due"
              data-testid="invoice-due-days"
              inputMode="numeric"
              placeholder="Chargebee's site default"
              value={dueDays}
              onChange={(e) => setDueDays(e.target.value)}
            />
          </div>

          <label className="flex items-start gap-2 text-sm text-muted-foreground">
            <input
              type="checkbox"
              className="mt-0.5"
              data-testid="invoice-force-new"
              checked={forceNew}
              disabled={busy}
              onChange={(e) => {
                const checked = e.target.checked;
                setForceNew(checked);
                setForceNewNonce(checked ? crypto.randomUUID() : undefined);
                setForceNewAttempted(false);
                if (!checked) clearUnresolvedForceNew(scope);
              }}
            />
            <span>
              This is a deliberate duplicate — send it as a new invoice, not a retry of an earlier
              one for the same customer and amount.
            </span>
          </label>

          {live ? (
            <Alert variant="destructive" data-testid="invoice-live-warning">
              <TriangleAlert className="size-4" />
              <AlertDescription>
                <code>{site}</code> does not look like a Chargebee test site. This will bill a real
                customer real money.
              </AlertDescription>
            </Alert>
          ) : null}

          {ready ? (
            <p className="text-sm" data-testid="invoice-confirm-line">
              Invoice <strong>{email.trim()}</strong>{" "}
              <strong>{fromMinorUnits(minor ?? 0, currency)}</strong> on{" "}
              <strong>
                {site ?? "the connected site"}
                {live ? " (live)" : ""}
              </strong>
              .
            </p>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={onSubmit} disabled={!ready || busy} data-testid="invoice-send">
            {busy ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
            Send invoice
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
