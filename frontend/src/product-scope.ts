// Surfaces hidden while the product is scoped to one company per install.
//
// Every flag here hides a control; none of them changes a stored value or a
// server path. `ComposioMode::Managed`, the inference `managed` legacy alias
// and the hosts registry all still exist and still resolve exactly as before,
// so a company already on a hidden setting keeps working and re-enabling a
// surface is a single edit in this file.

/** Hides the host roster, "Add a host" and "Manage hosts" in the switcher. */
export const HOSTS_HIDDEN = true;

/** Hides company switching, "All companies…" and "New company". */
export const COMPANY_SWITCHING_HIDDEN = true;

/** Hides the wizard's Advanced → Host group (bind address, workspace quotas). */
export const HOST_SETTINGS_HIDDEN = true;

/** Hides the OpenHuman-managed Composio route, leaving BYOK the only choice. */
export const COMPOSIO_MANAGED_HIDDEN = true;

/** Hides the managed inference provider, leaving the operator to name one. */
export const INFERENCE_MANAGED_HIDDEN = true;
