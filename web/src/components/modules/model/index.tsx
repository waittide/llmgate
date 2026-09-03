import { useMemo } from 'react';
import { useModelList } from '@/api/model';
import { PageActions, usePageActionsStore } from '@/components/common/PageActions';
import { ModelItem } from './Item';
import { CreateDialogContent } from './Create';
import { VirtualizedGrid } from '@/components/common/VirtualizedGrid';

// ModelActions 向稳定顶栏提供模型页面的搜索、视图选项和创建入口。
export function ModelActions() {
    return (
        <PageActions page="model">
            <CreateDialogContent />
        </PageActions>
    );
}

// Model 渲染模型列表正文。
export function Model() {
    const { data: models } = useModelList();
    const searchTerm = usePageActionsStore((state) => state.searchTerms.model || '');
    const layout = usePageActionsStore((state) => state.layouts.model || 'grid');
    const sortOrder = usePageActionsStore((state) => state.sortOrders.model === 'desc' ? 'desc' : 'asc');
    const filter = usePageActionsStore((state) => state.modelFilter);

    const sortedModels = useMemo(() => {
        if (!models) return [];
        return [...models].sort((a, b) =>
            sortOrder === 'asc' ? a.name.localeCompare(b.name) : b.name.localeCompare(a.name)
        );
    }, [models, sortOrder]);

    const visibleModels = useMemo(() => {
        const term = searchTerm.toLowerCase().trim();
        const byName = !term ? sortedModels : sortedModels.filter((m) => m.name.toLowerCase().includes(term));
        const hasPricing = (model: (typeof byName)[number]) =>
            model.input + model.output + model.cache_read + model.cache_write > 0;

        if (filter === 'priced') {
            return byName.filter(hasPricing);
        }
        if (filter === 'free') {
            return byName.filter((m) => !hasPricing(m));
        }

        return byName;
    }, [sortedModels, searchTerm, filter]);

    return (
        <VirtualizedGrid
            items={visibleModels}
            layout={layout}
            columns={{ default: 1, md: 2, lg: 3 }}
            estimateItemHeight={112}
            getItemKey={(model) => `model-${model.name}`}
            renderItem={(model) => <ModelItem model={model} layout={layout} />}
        />
    );
}
