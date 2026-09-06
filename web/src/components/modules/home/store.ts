import { reactive } from 'vue';

// 首页各处共用的统计维度: 金额, 次数, 词元。
export type MetricKey = 'cost' | 'count' | 'tokens';

// 趋势图可选的时间周期, 单位为天。
export type ChartPeriod = '1' | '7' | '30';

// 首页各区块的视图选项, 除渠道名模糊开关外均持久化到 localStorage。
export interface HomeViewState {
    channelRankSortMode: MetricKey; // 渠道排行榜的排序维度。
    modelRankSortMode: MetricKey; // 模型排行榜的排序维度。
    chartMetricType: MetricKey; // 趋势图展示的指标。
    chartPeriod: ChartPeriod; // 趋势图的时间周期。
    isChannelNameHidden: boolean; // 是否模糊渠道名称, 分享图跟随此状态, 不持久化。
    setChannelRankSortMode: (value: MetricKey) => void;
    setModelRankSortMode: (value: MetricKey) => void;
    setChartMetricType: (value: MetricKey) => void;
    setChartPeriod: (value: ChartPeriod) => void;
    setChannelNameHidden: (value: boolean) => void;
}

const STORAGE_KEY = 'home-view-options-storage';

function readPersistedHomeState(): Partial<HomeViewState> {
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

function persistHomeState() {
    if (typeof window === 'undefined') return;
    try {
        const stateToSave = {
            channelRankSortMode: homeViewStore.channelRankSortMode,
            modelRankSortMode: homeViewStore.modelRankSortMode,
            chartMetricType: homeViewStore.chartMetricType,
            chartPeriod: homeViewStore.chartPeriod,
        };
        localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: stateToSave }));
    } catch {
        // ignore storage write errors
    }
}

const saved = readPersistedHomeState();

export const homeViewStore = reactive<HomeViewState>({
    channelRankSortMode: saved.channelRankSortMode || 'cost',
    modelRankSortMode: saved.modelRankSortMode || 'cost',
    chartMetricType: saved.chartMetricType || 'cost',
    chartPeriod: saved.chartPeriod || '1',
    isChannelNameHidden: false,

    setChannelRankSortMode(value: MetricKey) {
        homeViewStore.channelRankSortMode = value;
        persistHomeState();
    },

    setModelRankSortMode(value: MetricKey) {
        homeViewStore.modelRankSortMode = value;
        persistHomeState();
    },

    setChartMetricType(value: MetricKey) {
        homeViewStore.chartMetricType = value;
        persistHomeState();
    },

    setChartPeriod(value: ChartPeriod) {
        homeViewStore.chartPeriod = value;
        persistHomeState();
    },

    setChannelNameHidden(value: boolean) {
        homeViewStore.isChannelNameHidden = value;
    },
});

export function useHomeViewStore<T = HomeViewState>(
    selector?: (state: HomeViewState) => T,
): T {
    if (selector) {
        return selector(homeViewStore);
    }
    return homeViewStore as unknown as T;
}
