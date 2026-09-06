import { reactive } from 'vue';

export type PageActionPage = 'channel' | 'group' | 'model';
export type PageActionLayout = 'grid' | 'list';
export type PageActionSortOrder = 'asc' | 'desc';
export type ChannelFilter = 'all' | 'enabled' | 'disabled';
export type GroupFilter = 'all' | 'with-members' | 'empty';
export type ModelFilter = 'all' | 'priced' | 'free';

export interface PageActionsState {
    searchTerms: Partial<Record<PageActionPage, string>>;
    layouts: Partial<Record<PageActionPage, PageActionLayout>>;
    sortOrders: Partial<Record<PageActionPage, PageActionSortOrder>>;
    channelFilter: ChannelFilter;
    groupFilter: GroupFilter;
    modelFilter: ModelFilter;
    setSearchTerm: (page: PageActionPage, value: string) => void;
    setLayout: (page: PageActionPage, value: PageActionLayout) => void;
    setSort: (page: PageActionPage, order: PageActionSortOrder) => void;
    setChannelFilter: (value: ChannelFilter) => void;
    setGroupFilter: (value: GroupFilter) => void;
    setModelFilter: (value: ModelFilter) => void;
}

const STORAGE_KEY = 'page-actions-storage';

function readPersistedState(): Partial<PageActionsState> {
    if (typeof window === 'undefined') return {};
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        return parsed?.state || {};
    } catch {
        return {};
    }
}

function persistState() {
    if (typeof window === 'undefined') return;
    try {
        const stateToSave = {
            layouts: pageActionsStore.layouts,
            sortOrders: pageActionsStore.sortOrders,
            channelFilter: pageActionsStore.channelFilter,
            groupFilter: pageActionsStore.groupFilter,
            modelFilter: pageActionsStore.modelFilter,
        };
        localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: stateToSave }));
    } catch {
        // ignore storage write errors
    }
}

const saved = readPersistedState();

export const pageActionsStore = reactive<PageActionsState>({
    searchTerms: {},
    layouts: saved.layouts || {},
    sortOrders: saved.sortOrders || {},
    channelFilter: saved.channelFilter || 'all',
    groupFilter: saved.groupFilter || 'all',
    modelFilter: saved.modelFilter || 'all',

    setSearchTerm(page: PageActionPage, value: string) {
        pageActionsStore.searchTerms[page] = value;
    },

    setLayout(page: PageActionPage, value: PageActionLayout) {
        pageActionsStore.layouts[page] = value;
        persistState();
    },

    setSort(page: PageActionPage, order: PageActionSortOrder) {
        pageActionsStore.sortOrders[page] = order;
        persistState();
    },

    setChannelFilter(value: ChannelFilter) {
        pageActionsStore.channelFilter = value;
        persistState();
    },

    setGroupFilter(value: GroupFilter) {
        pageActionsStore.groupFilter = value;
        persistState();
    },

    setModelFilter(value: ModelFilter) {
        pageActionsStore.modelFilter = value;
        persistState();
    },
});

export function usePageActionsStore<T = PageActionsState>(
    selector?: (state: PageActionsState) => T,
): T {
    if (selector) {
        return selector(pageActionsStore);
    }
    return pageActionsStore as unknown as T;
}
