# Security standards

What has to be true of a change before it goes anywhere near a customer's data.

| Rule | Why it is a rule and not a preference |
| --- | --- |
| Every client-supplied identifier is an authorization question | The most common real finding is a change that checks *who you are* and not *whether you may* |
| Secrets live in the secret store, never in a manifest or a note | A credential in a file is a credential in every backup of that file |
| Failure paths fail closed | A timeout that falls open is an outage that becomes a breach |
| Nothing about an unfixed finding goes anywhere public | The audience for a live vulnerability is this roster and the operator |
| A dependency advisory is triaged, not deferred | "Not exploitable here" is a finding with an argument, not a shrug |

Findings go on the `security-findings` ledger with the severity judged on what
it lets somebody *do*. Run the `security-review` skill whenever a change touches
auth, payments, uploads, or anything keeping one customer's data away from
another's — see [[Engineering standards]] and the [[Incident runbook]].
