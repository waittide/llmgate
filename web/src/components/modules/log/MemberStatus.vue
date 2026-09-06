<script setup lang="ts">
import { computed } from 'vue';
import { LuCheckCircle } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import type { Group } from '@/api/group';

export interface MemberStatusProps {
    group: Group;
    itemId?: number;
    now: number;
    active?: boolean;
    activeClassName?: string;
}

const props = withDefaults(defineProps<MemberStatusProps>(), {
    active: false,
    itemId: undefined,
    activeClassName: undefined,
});

const { t } = useTranslations('group.card');

const isCooling = computed(() => {
    if (props.group.mode === 'failover' && props.itemId !== undefined && props.group.runtime) {
        const cooldownUntil = props.group.runtime.cooldowns[props.itemId] ?? 0;
        return props.now < cooldownUntil;
    }
    return false;
});

const cooldownSeconds = computed(() => {
    if (props.itemId === undefined || !props.group.runtime) return 0;
    const cooldownUntil = props.group.runtime.cooldowns[props.itemId] ?? 0;
    return Math.ceil((cooldownUntil - props.now) / 1000);
});
</script>

<template>
    <div v-if="isCooling" class="flex items-center gap-1.5 shrink-0">
        <Badge
            variant="outline"
            class="shrink-0 px-1.5 py-0 text-[10px] font-medium border-orange-500/30 bg-orange-500/10 text-orange-600 dark:text-orange-400"
        >
            {{ t('cooling', { seconds: cooldownSeconds }) }}
        </Badge>
    </div>
    <span
        v-else-if="active"
        aria-hidden="true"
        :class="cn('inline-flex shrink-0 text-primary', activeClassName)"
    >
        <LuCheckCircle class="size-4" />
    </span>
</template>
