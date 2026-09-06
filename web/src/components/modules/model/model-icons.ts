import type { Component } from 'vue';
import {
    SiOpenai,
    SiAnthropic,
    SiGooglegemini,
    SiGoogle,
    SiXiaomi,
    SiMeta,
    SiBytedance,
    SiBaidu,
    SiNvidia,
    SiPerplexity,
    SiMicrosoft,
    SiAlibabacloud,
} from 'vue-icons-plus/si';
import { LuBot, LuBrain, LuCpu, LuSparkles } from 'vue-icons-plus/lu';

export type ModelIconConfig = {
    keywords: string[];
    Icon: Component;
    className?: string;
    color: string;
};

const MODEL_ICON_PATTERNS: ModelIconConfig[] = [
    { keywords: ['gpt-', 'o1', 'o3', 'o4', 'chatgpt', 'text-embedding', 'dall-e', 'openai'], Icon: SiOpenai, className: 'brightness-0 dark:invert', color: '#10A37F' },
    { keywords: ['claude', 'anthropic'], Icon: SiAnthropic, color: '#D7765A' },
    { keywords: ['gemini'], Icon: SiGooglegemini, color: '#4285F4' },
    { keywords: ['gemma'], Icon: SiGooglegemini, color: '#4285F4' },
    { keywords: ['palm', 'google'], Icon: SiGoogle, color: '#4285F4' },
    { keywords: ['xiaomi', 'mimo'], Icon: SiXiaomi, color: '#FF6900' },
    { keywords: ['deepseek'], Icon: LuBrain, color: '#4D6BFE' },
    { keywords: ['grok', 'xai'], Icon: LuSparkles, color: '#000000' },
    { keywords: ['qwen', 'qwq', 'alibaba'], Icon: SiAlibabacloud, className: 'brightness-0 dark:invert', color: '#6B4EFF' },
    { keywords: ['glm', 'chatglm', 'zhipu', 'z-ai'], Icon: LuCpu, color: '#3C5BFC' },
    { keywords: ['minimax', 'abab'], Icon: LuBot, color: '#1A1A2E' },
    { keywords: ['moonshot', 'kimi'], Icon: LuSparkles, color: '#000000' },
    { keywords: ['mistral', 'mixtral', 'codestral', 'pixtral'], Icon: LuBrain, color: '#F7D046' },
    { keywords: ['llama', 'meta-llama', 'meta'], Icon: SiMeta, color: '#0668E1' },
    { keywords: ['doubao', 'skylark', 'bytedance'], Icon: SiBytedance, color: '#00D6C2' },
    { keywords: ['yi-', '01-ai'], Icon: LuSparkles, color: '#1B1464' },
    { keywords: ['hunyuan'], Icon: LuCpu, color: '#0052D9' },
    { keywords: ['spark'], Icon: LuSparkles, color: '#0078FF' },
    { keywords: ['ernie', 'wenxin', 'baidu'], Icon: SiBaidu, color: '#2932E1' },
    { keywords: ['internlm'], Icon: LuBrain, color: '#2F54EB' },
    { keywords: ['stepfun', 'step-'], Icon: LuSparkles, color: '#5B5CFF' },
    { keywords: ['nvidia', 'nemotron'], Icon: SiNvidia, color: '#76B900' },
    { keywords: ['cohere', 'command'], Icon: LuCpu, color: '#39594D' },
    { keywords: ['perplexity'], Icon: SiPerplexity, color: '#20B8CD' },
    { keywords: ['phi-'], Icon: SiMicrosoft, color: '#00BCF2' },
    { keywords: ['kat'], Icon: LuSparkles, color: '#1969FC' },
];

const DEFAULT_CONFIG: { Icon: Component; className?: string; color: string } = {
    Icon: SiOpenai,
    className: 'brightness-0 dark:invert',
    color: '#10A37F',
};

export function getModelIcon(modelName: string): { Icon: Component; className?: string; color: string } {
    const lowerName = modelName.toLowerCase();
    for (const { keywords, Icon, className, color } of MODEL_ICON_PATTERNS) {
        if (keywords.some((keyword) => lowerName.includes(keyword))) {
            return { Icon, className, color };
        }
    }
    return DEFAULT_CONFIG;
}
