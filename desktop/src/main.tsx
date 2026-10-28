import React, { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import './style.css';

type Config={language:string;mode:string;asr_model:string;auto_insert:boolean;restore_text_clipboard:boolean;delete_audio_on_success:boolean;mic_node:string;max_record_seconds:number;service_url:string};
type Device={node:string;label:string;virtual:boolean;default:boolean};
type DeviceInfo={devices:Device[];default?:string|null;active?:string|null;configured?:string;error?:string|null};
type Last={available:boolean;text:string;raw:string;warning:string;phase:string;last_job:string};
type Status={state:{phase:string;error?:string;last_result?:string};doctor:{session?:string;desktop?:string;mic_node?:string;eitype?:boolean;has_groq_key?:boolean;service_url?:string;service_mode?:boolean;has_service_token?:boolean;checks?:{id:string;status:string;detail:string}[]}};
const langs:Record<string,string>={auto:'自动检测',en:'English',zh:'中文',ja:'日本語',ko:'한국어',es:'Español',fr:'Français',de:'Deutsch',pt:'Português',it:'Italiano',ru:'Русский',ar:'العربية',hi:'हिन्दी',tr:'Türkçe',nl:'Nederlands',pl:'Polski',id:'Bahasa Indonesia',vi:'Tiếng Việt',th:'ไทย',sv:'Svenska',uk:'Українська'};
const titles:Record<string,string>={idle:'准备就绪',recording:'录音中',processing:'正在转写',injecting:'正在输入',error:'需要处理'};
function App(){
 const [status,setStatus]=useState<Status|null>(null);
 const [config,setConfig]=useState<Config|null>(null);
 const [devices,setDevices]=useState<DeviceInfo|null>(null);
 const [last,setLast]=useState<Last|null>(null);
 const [showRaw,setShowRaw]=useState(false);
 const [error,setError]=useState('');
 const [saved,setSaved]=useState(false);
 const refresh=useCallback(async()=>{
   try{setStatus(await invoke<Status>('get_status'));setLast(await invoke<Last>('get_last'));}
   catch(e){setError(String(e));}
 },[]);
 const loadDevices=useCallback(async()=>{try{setDevices(await invoke<DeviceInfo>('get_devices'));}catch(e){setError(String(e));}},[]);
 useEffect(()=>{refresh();loadDevices();invoke<Config>('get_settings').then(setConfig).catch(e=>setError(String(e)));
   const id=setInterval(refresh,1600);return()=>clearInterval(id);},[refresh,loadDevices]);
 async function act(name:string){try{setError('');await invoke('action',{name});await refresh();}catch(e){setError(String(e));}}
 async function clearLast(){try{setError('');await invoke('clear_last');setLast(await invoke<Last>('get_last'));}catch(e){setError(String(e));}}
 async function change<K extends keyof Config>(key:K,value:Config[K]){if(!config)return;setConfig({...config,[key]:value});setSaved(false);try{await invoke('set_setting',{key,value:String(value)});setSaved(true);}catch(e){setError(String(e));}}
 const phase=status?.state.phase||'unknown';
 const text=showRaw?(last?.raw||''):(last?.text||'');
 const knownNodes=new Set((devices?.devices||[]).map(d=>d.node));
 return <div className="page">
  <header><div className="brand"><div className="logo">S.</div><div><b>scribiva.</b><small>VOICE-FIRST WRITING</small></div></div><div className="chip">LINUX ALPHA</div></header>
  <h1>Speak naturally.<br/><span>Write seamlessly.</span></h1>
  <p className="subtitle">全局语音输入 · 多语言听写 · 自动语法修正</p>
  <main>
   <section className="panel"><h2>语音输入</h2><div className="status"><i className={phase}/><div><div className="statusName">{titles[phase]||phase}</div><small>{status?.state.error||'保持目标应用光标位置，按快捷键即可输入'}</small></div></div>
    <div className="actions"><button onClick={()=>act('toggle')}>开始 / 结束录音</button><button className="soft" onClick={()=>act('cancel')}>取消</button><button className="soft" onClick={()=>act('retry')}>重试</button><button className="soft" onClick={()=>act('discard')}>丢弃</button></div>
    <p className="hint">快捷键 Ctrl + Alt + V · Obsidian Live Preview / Source Mode</p></section>
   <section className="panel"><h2>听写设置</h2>{config?<>
    <label>听写语言<select value={config.language} onChange={e=>change('language',e.target.value)}>{Object.entries(langs).map(([id,name])=><option key={id} value={id}>{name}</option>)}</select></label>
    <label>AI 处理模式<select value={config.mode} onChange={e=>change('mode',e.target.value)}><option value="raw">原文</option><option value="standard">智能修正</option><option value="polished">流畅表达</option></select></label>
    <label>识别模型<select value={config.asr_model} onChange={e=>change('asr_model',e.target.value)}><option value="whisper-large-v3">Whisper Large V3</option><option value="whisper-large-v3-turbo">Whisper Large V3 Turbo</option></select></label>
    <label>麦克风<select value={config.mic_node} onChange={e=>change('mic_node',e.target.value)}><option value="">自动 · 系统默认{devices?.default?`（${devices.default}）`:''}</option>
      {(devices?.devices||[]).map(d=><option key={d.node} value={d.node}>{d.label}{d.virtual?' · 虚拟':''}{d.default?' · 默认':''}</option>)}
      {config.mic_node&&!knownNodes.has(config.mic_node)&&<option value={config.mic_node}>{config.mic_node}（当前配置，未检测到）</option>}
     </select></label>
    <label>最长录音时长（秒）<input type="number" min={5} max={600} step={5} value={config.max_record_seconds} onChange={e=>{const v=Number(e.target.value);if(Number.isFinite(v)&&v>=5&&v<=600)change('max_record_seconds',v);}}/></label>
    <label>订阅服务地址<input type="url" defaultValue={config.service_url} placeholder="https://api.example.com（留空=直接模式）" onBlur={e=>change('service_url',e.target.value.trim())}/></label>
    <small className="deviceInfo">服务令牌请运行 <code>scribiva service login --url …</code> 配置，不会显示在界面中。</small>
    <label className="check"><input type="checkbox" checked={config.auto_insert} onChange={e=>change('auto_insert',e.target.checked)}/> 自动插入当前输入框</label>
    <label className="check"><input type="checkbox" checked={config.restore_text_clipboard} onChange={e=>change('restore_text_clipboard',e.target.checked)}/> 尝试恢复原有纯文本剪贴板</label>
    <label className="check"><input type="checkbox" checked={config.delete_audio_on_success} onChange={e=>change('delete_audio_on_success',e.target.checked)}/> 成功后删除本地录音</label>
    {saved&&<small className="success">设置已保存</small>}
    <small className="deviceInfo">当前生效麦克风：{devices?.active||'未配置'}{devices?.error?` · 扫描警告：${devices.error}`:''}</small>
   </>:<small>读取设置中…</small>}</section>
   <section className="panel wide"><div className="row"><h2>最近一次结果</h2><div className="actions"><button className="soft" disabled={!last?.text} onClick={()=>act('copy-last')}>复制</button><button className="soft" disabled={!last?.available} onClick={()=>setShowRaw(!showRaw)}>{showRaw?'查看修正后文本':'查看 ASR 原文'}</button><button className="soft" disabled={!last?.available} onClick={clearLast}>清除本地结果</button></div></div>
    <pre className="lastText">{last?.available?(text||'没有保存该版本文本'):'尚无结果'}</pre>
    {last?.warning&&<small className="warning">{last.warning}</small>}
    <small className="deviceInfo">结果仅保存在本机 · {last?.last_job?`任务 ${last.last_job} · `:''}状态 {last?.phase||'-'}</small>
   </section>
   <section className="panel wide"><h2>环境诊断</h2><p className="diagnostic">显示协议：{status?.doctor.session??'-'} · 桌面：{status?.doctor.desktop??'-'}<br/>麦克风：{status?.doctor.mic_node??'未配置'}<br/>eitype：{status?.doctor.eitype?'已安装':'未检测到'} · {status?.doctor.service_mode?`服务：${status.doctor.service_url||'-'} · 令牌${status.doctor.has_service_token?'已配置':'未配置'}`:`Groq Key：${status?.doctor.has_groq_key?'已配置':'未配置'}`}</p>
    <div className="checks">{(status?.doctor.checks||[]).map(c=><div key={c.id} className="checkItem"><b>{c.id}<span className={`state ${c.status}`}>{c.status}</span></b><span>{c.detail}</span></div>)}</div>
    <div className="notice">开发 Alpha：直接模式会把音频发送至 Groq；配置订阅服务后由自建 API 持有密钥并执行额度控制。当前仅提供 Linux 核心，Windows 原生适配尚未实现。</div></section>
  </main>
  {!!error&&<aside role="alert">{error}</aside>}
 </div>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App/></React.StrictMode>);
