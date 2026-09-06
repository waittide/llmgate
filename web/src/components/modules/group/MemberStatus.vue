<script setup lang="ts">
import { computed, ref, watchEffect, toValue, type MaybeRefOrGetter } from 'vue';
import { LuCheckCircle2 } from 'vue-icons-plus/lu';
import { Badge } from '@/components/ui/badge';
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';
import type { Group } from '@/api/group';

export interface MemberStatusProps {
    group: Group; // group 提供当前路由和成员冷却时间戳。
    itemId?: number; // itemId 是待展示状态的成员 ID。
    now: number; // now 是所属列表共享的当前 Unix 毫秒时间。
    disabled?: boolean; // disabled 表示成员所属渠道已被禁用(禁用态优先于冷却展示)。
    active?: boolean; // active 表示该成员当前正在使用。
    activeClassName?: string; // activeClassName 调整原有选中图标在不同列表中的间距。
}

const props = withDefaults(defineProps<MemberStatusProps>(), {
    active: false,
    activeClassName: undefined,
});

const { t } = useTranslations('group');

const cooldownUntil = computed(() => {
    if (props.group.mode !== 'failover' || props.itemId === undefined || !props.group.runtime) {
        return 0;
    }
    return props.group.runtime.cooldowns[props.itemId] ?? 0;
});

const isDisabled = computed(() => props.disabled === true);
const isCooling = computed(() => !isDisabled.value && props.now < cooldownUntil.value);

const coolingLabel = computed(() => {
    const seconds = Math.ceil((cooldownUntil.value - props.now) / 1000);
    return t('card.cooling', { seconds });
});
</script>

<script lang="ts">
// useRuntimeClock 为一个或多个分组提供共享倒计时当前时间。
export function useRuntimeClock(source?: MaybeRefOrGetter<Group | Group[] | undefined>) {
    const now = ref(Date.now());

    watchEffect((onCleanup) => {
        const raw = toValue(source);
        const groups = raw === undefined ? [] : Array.isArray(raw) ? raw : [raw];
        let enabled = false;
        let lastDeadline = 0;

        for (const group of groups) {
            if (group.mode !== 'failover' || !group.runtime) continue;
            enabled = true;
            for (const deadline of Object.values(group.runtime.cooldowns)) {
                lastDeadline = Math.max(lastDeadline, deadline);
            }
        }

        if (!enabled) return;

        let timer = 0;
        const tick = () => {
            const next = Date.now();
            now.value = next;
            if (next >= lastDeadline) window.clearInterval(timer);
        };

        const immediate = window.setTimeout(tick, 0);
        if (Date.now() < lastDeadline) timer = window.setInterval(tick, 1000);

        onCleanup(() => {
            window.clearTimeout(immediate);
            window.clearInterval(timer);
        });
    });

    return now;
}
</script>

<template>
    <div v-if="isDisabled" class="flex items-center gap-1.5 shrink-0">
        <Badge
            variant="outline"
            class="shrink-0 px-1.5 py-0 text-[10px] font-medium border-muted-foreground/30 bg-muted-foreground/10 text-muted-foreground"
        >
            {{ t('card.disabled') }}
        </Badge>
    </div>
    <div v-else-if="isCooling" class="flex items-center gap-1.5 shrink-0">
        <Badge
            variant="outline"
            class="shrink-0 px-1.5 py-0 text-[10px] font-medium border-orange-500/30 bg-orange-500/10 text-orange-600 dark:text-orange-400"
        >
            {{ coolingLabel }}
        </Badge>
    </div>
    <span
        v-else-if="active"
        aria-hidden="true"
        :class="cn('inline-flex shrink-0 text-primary', activeClassName)"
    >
        <LuCheckCircle2 class="size-4" />
    </span>
</template>
