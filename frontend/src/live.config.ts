import { defineLiveCollection } from 'astro:content';
import { secAgreementsLoader } from './loaders/sec-api';

// Request-time collection: fresh on every request, no rebuild needed.
const agreements = defineLiveCollection({
  loader: secAgreementsLoader(),
});

export const collections = { agreements };
