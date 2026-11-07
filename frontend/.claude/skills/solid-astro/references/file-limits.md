---
name: file-limits
applies-to: "**/*.astro"
---

# File Size Limits in Astro

## The Rule

**Source files stay within `FUSE_SOLID_MAX_LINES`. Split by responsibility when cohesion drops.**

This is not a stylistic preference — it enforces Single Responsibility and prevents "god components." The hook's ceiling is the only size authority.

## Split Strategy by File Type

| File Type | Action when cohesion drops |
|-----------|----------------------------|
| Page (`.astro` in `pages/`) | Extract components |
| Layout component | Extract sections |
| UI component | Split into sub-components |
| Service / utility | Extract helper functions |
| Content schema | Split into sub-schemas |
| Interface file | Split by domain |

## How to Split a Page

```astro
<!-- BAD: page doing everything -->
<!-- src/pages/index.astro -->

<!-- GOOD: Page as composition of focused components -->
---
// src/pages/index.astro — composition only
import HeroSection from '../components/home/HeroSection.astro';
import FeaturesGrid from '../components/home/FeaturesGrid.astro';
import TestimonialsSlider from '../components/home/TestimonialsSlider.astro';
import CtaSection from '../components/home/CtaSection.astro';
import BaseLayout from '../layouts/BaseLayout.astro';
import { getHomeData } from '../lib/home';

const { features, testimonials } = await getHomeData();
---

<BaseLayout title="Home">
  <HeroSection />
  <FeaturesGrid features={features} />
  <TestimonialsSlider testimonials={testimonials} />
  <CtaSection />
</BaseLayout>
```

## How to Split a Large Service

```typescript
// BAD: src/lib/blog.ts — queries, transforms and exports mixed in one file

// GOOD: Split by responsibility
// src/lib/blog/queries.ts    — database/API queries
// src/lib/blog/transforms.ts — data transformation
// src/lib/blog/index.ts      — public API re-exports
```

## Automated Check

```bash
# Check files exceeding the hook ceiling (FUSE_SOLID_MAX_LINES)
find src -name "*.astro" -o -name "*.ts" -o -name "*.tsx" \
  | xargs wc -l | awk -v max="${FUSE_SOLID_MAX_LINES:-100}" '$1 > max {print $0}' | grep -v total
```
