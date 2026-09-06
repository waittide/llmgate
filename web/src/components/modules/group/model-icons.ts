import type { Component } from 'vue';
import { OpenAIIcon, ClaudeIcon, GeminiIcon, DeepSeekIcon, MistralIcon, QwenIcon, MetaIcon, CohereIcon, PerplexityIcon, ZhipuIcon, YiIcon, KimiIcon, MinimaxIcon, DoubaoIcon, HunyuanIcon, SparkIcon, WenxinIcon, NvidiaIcon, GrokIcon, GoogleIcon, InternLMIcon, StepfunIcon, GemmaIcon, MicrosoftIcon, KwaiKATIcon, XiaomiMimoIcon } from '@/components/brand-icons';

export type ModelIconConfig = {
    keywords: string[];
    Icon: Component;
    className?: string;
    color: string;
};

const MODEL_ICON_PATTERNS: ModelIconConfig[] = [
    { keywords: ['gpt-', 'o1', 'o3', 'o4', 'chatgpt', 'text-embedding', 'dall-e', 'openai'], Icon: OpenAIIcon, className: 'brightness-0 dark:invert', color: '#10A37F' },
    { keywords: ['claude', 'anthropic'], Icon: ClaudeIcon, color: '#D7765A' },
    { keywords: ['gemini'], Icon: GeminiIcon, color: '#4285F4' },
    { keywords: ['gemma'], Icon: GemmaIcon, color: '#4285F4' },
    { keywords: ['palm', 'google'], Icon: GoogleIcon, color: '#4285F4' },
    { keywords: ['xiaomi', 'mimo'], Icon: XiaomiMimoIcon, color: '#FF6900' },
    { keywords: ['deepseek'], Icon: DeepSeekIcon, color: '#4D6BFE' },
    { keywords: ['grok', 'xai'], Icon: GrokIcon, color: '#000000' },
    { keywords: ['qwen', 'qwq', 'alibaba'], Icon: QwenIcon, className: 'brightness-0 dark:invert', color: '#6B4EFF' },
    { keywords: ['glm', 'chatglm', 'zhipu', 'z-ai'], Icon: ZhipuIcon, color: '#3C5BFC' },
    { keywords: ['minimax', 'abab'], Icon: MinimaxIcon, color: '#1A1A2E' },
    { keywords: ['moonshot', 'kimi'], Icon: KimiIcon, color: '#000000' },
    { keywords: ['mistral', 'mixtral', 'codestral', 'pixtral'], Icon: MistralIcon, color: '#F7D046' },
    { keywords: ['llama', 'meta-llama', 'meta'], Icon: MetaIcon, color: '#0668E1' },
    { keywords: ['doubao', 'skylark', 'bytedance'], Icon: DoubaoIcon, color: '#00D6C2' },
    { keywords: ['yi-', '01-ai'], Icon: YiIcon, color: '#1B1464' },
    { keywords: ['hunyuan'], Icon: HunyuanIcon, color: '#0052D9' },
    { keywords: ['spark'], Icon: SparkIcon, color: '#0078FF' },
    { keywords: ['ernie', 'wenxin', 'baidu'], Icon: WenxinIcon, color: '#2932E1' },
    { keywords: ['internlm'], Icon: InternLMIcon, color: '#2F54EB' },
    { keywords: ['stepfun', 'step-'], Icon: StepfunIcon, color: '#5B5CFF' },
    { keywords: ['nvidia', 'nemotron'], Icon: NvidiaIcon, color: '#76B900' },
    { keywords: ['cohere', 'command'], Icon: CohereIcon, color: '#39594D' },
    { keywords: ['perplexity'], Icon: PerplexityIcon, color: '#20B8CD' },
    { keywords: ['phi-'], Icon: MicrosoftIcon, color: '#00BCF2' },
    { keywords: ['kat'], Icon: KwaiKATIcon, color: '#1969FC' },
];

const DEFAULT_CONFIG = { Icon: OpenAIIcon, className: 'brightness-0 dark:invert', color: '#10A37F' };

export function getModelIcon(modelName: string): { Icon: Component; className?: string; color: string } {
    const lowerName = modelName.toLowerCase();
    for (const { keywords, Icon, className, color } of MODEL_ICON_PATTERNS) {
        if (keywords.some((keyword) => lowerName.includes(keyword))) {
            return { Icon, className, color };
        }
    }
    return DEFAULT_CONFIG;
}
