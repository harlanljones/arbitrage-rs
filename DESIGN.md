---
name: arbkit Results Ledger
description: An audited proof ledger for pessimistic, reproducible arbitrage evidence.
colors:
  paper: "#f3efe5"
  paper-deep: "#e7dfcf"
  ink: "#171814"
  muted-ink: "#5d5b52"
  audit-rule: "#aaa291"
  soft-rule: "#d7d0c0"
  verification: "#2f6d43"
  verification-soft: "#dbe7d9"
  loss-budget: "#9a4d2f"
  comparison: "#315a78"
typography:
  display:
    fontFamily: "Newsreader Variable, Georgia, serif"
    fontSize: "clamp(6rem, 12vw, 10rem)"
    fontWeight: 520
    lineHeight: 0.78
    letterSpacing: "-0.035em"
  headline:
    fontFamily: "Newsreader Variable, Georgia, serif"
    fontSize: "clamp(2.65rem, 5vw, 5.25rem)"
    fontWeight: 480
    lineHeight: 0.96
    letterSpacing: "-0.035em"
  title:
    fontFamily: "Newsreader Variable, Georgia, serif"
    fontSize: "clamp(2rem, 3.4vw, 3.25rem)"
    fontWeight: 520
    lineHeight: 1
    letterSpacing: "-0.025em"
  body:
    fontFamily: "IBM Plex Sans Variable, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.65
  label:
    fontFamily: "IBM Plex Mono, monospace"
    fontSize: "0.68rem"
    fontWeight: 400
    lineHeight: 1
    letterSpacing: "0.1em"
  data:
    fontFamily: "IBM Plex Mono, monospace"
    fontSize: "0.76rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  square: "0"
  binder-mark: "50%"
spacing:
  mobile-gutter: "20px"
  compact: "28px"
  ledger-gutter: "48px"
  section-block: "clamp(72px, 9vw, 132px)"
components:
  button-ledger:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.data}"
    rounded: "{rounded.square}"
    padding: "13px 18px"
  button-ledger-hover:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.paper}"
    typography: "{typography.data}"
    rounded: "{rounded.square}"
    padding: "13px 18px"
  select-run:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.data}"
    rounded: "{rounded.square}"
    padding: "12px 38px 12px 14px"
  ledger-result:
    backgroundColor: "{colors.verification-soft}"
    textColor: "{colors.verification}"
    rounded: "{rounded.square}"
    padding: "28px 34px"
---

# Design System: arbkit Results Ledger

## Overview

**Creative North Star: "The Audited Proof Ledger"**

The system turns benchmark claims into a sequence of inspectable evidence. It borrows the authority of an engineering ledger—warm uncoated paper, carbon ink, ruled partitions, binder marks, and disciplined data labels—while letting a few measured values reach editorial scale. The result should feel skeptical, precise, and materially grounded rather than like a generic analytics dashboard.

Persuasion comes from visible measurement boundaries, not decoration. A dominant result earns attention; provenance, methodology, units, comparison qualifiers, and semantic table equivalents earn trust. Verification green, comparison blue, and loss/budget copper remain functional annotations within an otherwise neutral field.

**Key Characteristics:**

- Editorial-scale measurements paired with compact monospaced evidence labels.
- Continuous ruled sections and unequal evidence fields instead of floating metric cards.
- Square controls, hairline borders, and flat paper layers with no decorative elevation.
- Semantic color reserved for verification, comparison, and budget or loss states.
- Explicit synthetic-workload, paper-trading, provenance, and cross-host boundaries.

## Colors

The palette is a restrained ledger field: warm paper and near-black carbon carry the page, with muted audit rules and three evidence colors used only when meaning requires them.

### Primary

- **Verification Green:** Marks selected measurements, passed verification, executable fills, positive comparison figures, and realized paper results.

### Secondary

- **Comparison Blue:** Identifies the non-selected host or comparison series without implying a same-machine trend.

### Tertiary

- **Loss & Budget Copper:** Marks latency budgets, phantom or decayed signals, focus outlines, issue numbering, and error state rules.

### Neutral

- **Warm Ledger Paper:** The canonical page and control ground.
- **Pressed Paper:** A darker tonal layer for ruler fields, scrollbar tracks, and unfilled execution tracks.
- **Carbon Ink:** Primary text, strong rules, chart axes, and inverted control states.
- **Soft Carbon:** Supporting copy, units, captions, and secondary metadata.
- **Audit Rule:** Structural partitions between major evidence fields.
- **Soft Rule:** Table rows, chart grids, and subordinate separators.
- **Verification Wash:** A quiet selected or successful result field behind verification green.

### Named Rules

**The Evidence-Only Color Rule.** Verification green, comparison blue, and loss/budget copper communicate data or state; they do not decorate neutral regions.

**The Paper Field Rule.** Warm ledger paper remains the dominant ground. Pressed paper and verification wash create only the tonal distinctions already present in the ledger.

## Typography

**Display Font:** Newsreader Variable (with Georgia and serif fallbacks)  
**Body Font:** IBM Plex Sans Variable (with sans-serif fallback)  
**Label/Mono Font:** IBM Plex Mono (with monospace fallback)

**Character:** Newsreader gives measurements and section claims editorial gravity; IBM Plex Sans keeps explanations direct; IBM Plex Mono turns labels, navigation, controls, units, and provenance into operational evidence. All three are self-hosted in the shipped dashboard.

### Hierarchy

- **Display** (520, fluid 6rem–10rem, 0.78 line-height): Reserved for the primary measured value, using lining and tabular numerals.
- **Headline** (480, fluid 2.65rem–5.25rem, 0.96 line-height): Leads major evidence sections with tight, balanced editorial lines.
- **Title** (520, fluid 2rem–3.25rem, 1 line-height): Carries evidence-rail figures and supporting measurements.
- **Body** (400, 1rem, 1.65 line-height): Explains methodology and limitations, generally capped near 68 characters per line.
- **Label** (400, 0.68rem, 0.1em letter-spacing, uppercase): Names evidence fields and table columns without competing with results.
- **Data** (400, 0.76rem, 1.5 line-height): Serves controls, tables, chart annotations, and metadata.

### Named Rules

**The Measurement Leads Rule.** Large Newsreader numerals state the result; nearby Plex Mono labels, units, and qualifiers define exactly what was measured.

**The Mono Means Evidence Rule.** Use the monospaced face for operational labels, controls, provenance, navigation, and chart annotation—not for explanatory paragraphs.

## Layout

The desktop shell is a centered ledger capped at 1600px, with a 72px binder rail and a 48px content gutter. The first viewport is intentionally asymmetric: a broad measurement field occupies roughly three quarters of the hero while the true logarithmic budget ruler anchors the narrower right field. Evidence then proceeds as one continuous vertical record separated by hairlines, with four-up or split grids only where comparison benefits from adjacency.

Section spacing is expansive and proportional; the normative section-block token controls the vertical cadence. At 1100px the binder rail and header navigation disappear, four-column ledgers collapse to two columns, and the content retains its ruled structure. At 760px the hero stacks, gutters reduce to the mobile-gutter token, evidence rails become single columns, financial and execution labels become vertical records, and wide history data remains horizontally scrollable rather than compressed beyond legibility.

**The Continuous Record Rule.** Major facts share borders and alignment in one evidence rail or ledger; do not split them into detached, equally weighted cards.

**The First-Viewport Proof Rule.** Preserve the pairing of the dominant p99 measurement, published-run control, scope stamp, and logarithmic budget ruler at every breakpoint, even when the ruler stacks below the measurement.

## Elevation & Depth

This system is flat. It uses no box shadows or decorative elevation. Depth comes from warm tonal layers, ruled boundaries, selected-row washes, and the physical suggestion of the binder rail. Hovered buttons invert ink and paper instead of lifting.

### Named Rules

**The Flat Ledger Rule.** Surfaces remain in the paper plane; hierarchy is created by scale, rules, alignment, and tonal fields, never shadow.

## Shapes

Controls, plot bars, tables, result fields, and containers use square corners. One-pixel rules define structure without rounded shells. Circular geometry is reserved for binder holes and plotted measurement dots; it is an evidence mark, not a container language.

**The Square Instrument Rule.** Buttons, selects, tooltip frames, bars, and content regions keep zero-radius corners.

## Components

### Buttons

- **Shape:** Square with a one-pixel carbon border.
- **Primary:** Paper ground with carbon text and compact ledger padding; used for download and retry actions.
- **Hover / Focus:** Hover inverts to carbon with paper text. Keyboard focus uses a three-pixel copper outline offset by four pixels.
- **Inline table action:** Borderless and underlined, retaining the same ink-inversion hover behavior inherited by buttons.

### Inputs / Fields

- **Style:** The published-run select is a native square field on ledger paper with a one-pixel carbon border, monospaced data text, and full-width behavior on mobile.
- **Focus:** Uses the shared copper focus outline rather than a glow or rounded ring.
- **State:** The field retains a native select indicator and exposes the complete dated environment label.

### Cards / Containers

- **Corner Style:** Square.
- **Background:** Ledger paper by default; pressed paper for the budget-ruler field; verification wash for successful or selected result cells.
- **Shadow Strategy:** None; see Elevation & Depth.
- **Border:** One-pixel carbon rules for major boundaries and audit or soft rules for subordinate partitions.
- **Internal Padding:** Desktop ledger cells generally use 28–34px; mobile cells contract to the mobile-gutter token.

### Navigation

The wordmark and section anchors use Plex Mono. Desktop anchors are quiet uppercase labels with an underline that draws from left to right over 180ms on hover or keyboard focus. Navigation disappears below 1100px while the wordmark and explicit synthetic/paper-trading scope stamp remain visible.

### Evidence Rail

Four unequal facts share a single ruled grid. Each fact pairs a compact uppercase mono label, a Newsreader value, and a muted qualifier. The rail collapses from four columns to two and then one without becoming a collection of cards.

### Budget Ruler

The signature graphic is an authored semantic SVG with a logarithmic nanoseconds-to-milliseconds scale. Carbon establishes the spine and ticks, copper locates the engineering budget, and verification green locates the selected p99. The figure includes a title, description, direct units, and caption.

### Charts and Verification Tables

Charts use fine carbon axes, soft dashed grid lines, direct units, square bars, and flat tooltips. The selected run is verification green, the comparison run is blue, and the distant budget is dashed copper. Latency and verification figures provide disclosure-based semantic tables; the history comparison is a native table with row and column headers.

### Motion

The only authored reveal is the loading-state evidence rule, which scans once over 1.2 seconds with a decisive ease-out curve. Navigation underlines transition over 180ms. Reduced-motion preferences collapse animation and transition durations and disable smooth scrolling.

## Do's and Don'ts

### Do:

- **Do** lead claims with dated measurements, visible units, and nearby provenance or methodology.
- **Do** preserve the large-measurement and logarithmic-ruler pairing as the dashboard's focal proof.
- **Do** use verification green, comparison blue, and loss/budget copper only for their implemented semantic roles.
- **Do** provide semantic HTML or table equivalents for graphical evidence.
- **Do** retain square corners, hairline partitions, and the continuous ledger reading order across breakpoints.
- **Do** keep synthetic workload, paper trading, and no-live-orders scope visible.

### Don't:

- **Don't** convert the evidence rail into floating metric cards or add decorative shadows.
- **Don't** present cross-host comparisons as same-machine performance trends.
- **Don't** use semantic evidence colors as broad decorative accents.
- **Don't** hide paper-trading, synthetic-data, measurement-boundary, or provenance qualifiers behind interaction.
- **Don't** replace the logarithmic budget ruler with a visually convenient linear scale.
- **Don't** round controls, bars, tooltips, or ledger containers.
