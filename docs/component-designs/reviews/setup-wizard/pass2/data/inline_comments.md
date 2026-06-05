<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 1348-1468 RULE: 13 SEVERITY: LOW -->
**Rule 13 — Version Everything**
The Appendix TypeScript types (`HardwareInfo`, `ModelScanResult`, `RecommendationResult`, `BinaryScanResult`) omit the `"version": 1` field that Section 11 mandates on all `/api/setup/*` responses. This creates ambiguity about whether `version` is part of the domain type or stripped by the `apiFetch` wrapper.

Suggested: Add a note to the Appendix clarifying that these are the domain payload shapes inside the versioned response envelope, e.g.:
```typescript
// All /api/setup/* responses are wrapped in { version: number, ...payload }.
// The types below represent the payload; the apiFetch wrapper validates
// and strips the version field before returning the typed result.
```

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 299-324 RULE: 11 SEVERITY: LOW -->
**Rule 11 — Make State Transitions Explicit**
The `STEP_TRANSITIONS` table's `canAdvance` predicates reference `s.hardware`, `s.binaryPath`, `s.selectedModels`, and `s.serverConfigs` — but the redesigned `SetupWizardStore` (earlier in this same section) holds only `selectedModelIds`, `userOverrides`, and `savePath`. Hardware, binary, and model data live in TanStack Query cache, not the store. The predicates would need access to the `QueryClient` to check those values.

Suggested: Update the predicate signature to accept both store and query client:
```typescript
canAdvance: (store: SetupWizardStore, queryClient: QueryClient) => boolean;
```
And update the hardware/binary predicates accordingly:
```typescript
hardware: {
  canAdvance: (_, qc) => qc.getQueryData(['setup', 'hardware']) != null,
  invalidatesOnReentry: ['configure', 'review']
},
binary: {
  canAdvance: (_, qc) => {
    const result = qc.getQueryData(['setup', 'scan-binary']);
    return result?.found === true;
  },
  invalidatesOnReentry: ['configure', 'review']
},
```
Alternatively, add a note that the predicates are pseudocode showing the logical intent rather than literal implementation.

---
