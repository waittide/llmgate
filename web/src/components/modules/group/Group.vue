<script setup lang="ts">
import { computed } from 'vue';
import { useGroupList, type Group } from '@/api/group';
import { pageActionsStore } from '@/components/common/page-actions-store';
import VirtualizedGrid from '@/components/common/VirtualizedGrid.vue';
import GroupCard from './Card.vue';
import { useRuntimeClock } from './MemberStatus.vue';

const { data: groups } = useGroupList(true, true);
const runtimeNow = useRuntimeClock(groups);

const sortedGroups = computed<Group[]>(() => {
    const list = groups.value ?? [];
    const sortOrder = pageActionsStore.sortOrders.group === 'desc' ? 'desc' : 'asc';
    return [...list].sort((a, b) =>
        sortOrder === 'asc' ? a.name.localeCompare(b.name) : b.name.localeCompare(a.name)
    );
});

const visibleGroups = computed<Group[]>(() => {
    const term = (pageActionsStore.searchTerms.group || '').toLowerCase().trim();
    const byName = !term
        ? sortedGroups.value
        : sortedGroups.value.filter((g) => g.name.toLowerCase().includes(term));

    const filter = pageActionsStore.groupFilter;
    if (filter === 'with-members') return byName.filter((g) => (g.items?.length || 0) > 0);
    if (filter === 'empty') return byName.filter((g) => (g.items?.length || 0) === 0);

    return byName;
});

function getGroupDeadline(group: Group): number {
    let deadline = 0;
    for (const cooldownUntil of Object.values(group.runtime?.cooldowns ?? {})) {
        deadline = Math.max(deadline, cooldownUntil);
    }
    return deadline > runtimeNow.value ? runtimeNow.value : deadline;
}
</script>

<template>
    <VirtualizedGrid
        :items="visibleGroups"
        :columns="{ default: 1, md: 2, lg: 3 }"
        :estimate-item-height="520"
        :get-item-key="(group, index) => group.id ?? `group-${index}`"
    >
        <template #default="{ item }">
            <GroupCard :group="item" :now="getGroupDeadline(item)" />
        </template>
    </VirtualizedGrid>
</template>
