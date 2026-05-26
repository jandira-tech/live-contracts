/**
 * Live Content Collections loader for SEC EX-10 agreements.
 *
 * Fetches at request time (no rebuild) so the homepage shows agreements the
 * moment the backend stores them. Backed by the internal API client in lib/api.
 */
import type { LiveLoader } from 'astro/loaders';
import { ex10Since, listEx10, ex10Detail, type Ex10Summary, type Ex10Detail } from '../lib/api';

export interface AgreementsCollectionFilter {
  /** When set, return only agreements found in the last N seconds. */
  seconds?: number;
  /** Otherwise, page through all agreements. */
  page?: number;
  pageSize?: number;
}

export interface AgreementEntryFilter {
  id: string | number;
}

type Data = Ex10Summary | Ex10Detail;

export function secAgreementsLoader(): LiveLoader<Data, AgreementEntryFilter, AgreementsCollectionFilter> {
  return {
    name: 'sec-agreements',

    async loadCollection({ filter }) {
      try {
        if (filter?.seconds) {
          const res = await ex10Since(filter.seconds);
          return {
            entries: res.items.map((item) => ({ id: String(item.id), data: item })),
            cacheHint: { lastModified: new Date() },
          };
        }
        const res = await listEx10(filter?.page ?? 1, filter?.pageSize ?? 20);
        return {
          entries: res.items.map((item) => ({ id: String(item.id), data: item })),
          cacheHint: { lastModified: new Date() },
        };
      } catch (error) {
        return { error: error instanceof Error ? error : new Error(String(error)) };
      }
    },

    async loadEntry({ filter }) {
      try {
        const detail = await ex10Detail(filter.id);
        if (!detail) return undefined;
        return { id: String(detail.id), data: detail };
      } catch (error) {
        return { error: error instanceof Error ? error : new Error(String(error)) };
      }
    },
  };
}
