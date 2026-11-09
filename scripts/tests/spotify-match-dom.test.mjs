import assert from 'node:assert/strict';
import { test } from 'node:test';
import { renderSpotifyMatch } from '../../.test-build/views/spotify-match.js';
import { installFakeDom, findAll, FakeEvent } from './fake-dom.mjs';
const track = {url:'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT',id:'4cOdK2wGLETKBW3PvgPWqT',title:'<script>song</script>',artists:['Artist'],duration:200,artwork:null};
const candidate = {videoId:'dQw4w9WgXcQ',url:'https://www.youtube.com/watch?v=dQw4w9WgXcQ',title:'Song',uploader:'Artist',duration:201,thumbnail:null,score:0.9,warnings:['Similar title']};
const state = (over={}) => ({stage:'choosing',url:track.url,track,query:'Artist Song',candidates:[candidate],selectedId:null,confirmed:null,warnings:[],error:null,...over});
const actions = (over={}) => ({setQuery(){},search(){},select(){},confirm(){},retry(){},...over});
test('review renders literal metadata and no Download or default selected candidate', () => {
 installFakeDom(); const view=renderSpotifyMatch(state(),actions());
 assert.ok(view.textContent.includes('<script>song</script>'));
 assert.equal(findAll(view,n=>n.tagName==='SCRIPT').length,0);
 assert.equal(findAll(view,n=>n.tagName==='INPUT' && n.getAttribute('type')==='radio').some(n=>n.checked),false);
 assert.equal(findAll(view,n=>n.tagName==='BUTTON').some(n=>n.textContent==='Download'),false);
 assert.equal(findAll(view,n=>n.tagName==='BUTTON' && n.textContent==='Use this match')[0].disabled,true);
});
test('select and confirm are separate actions; presentation never queues', () => {
 installFakeDom(); const selected=[]; let confirms=0;
 const view=renderSpotifyMatch(state({selectedId:candidate.videoId}),actions({select:id=>selected.push(id),confirm:()=>confirms++}));
 findAll(view,n=>n.tagName==='INPUT' && n.getAttribute('type')==='radio')[0].dispatchEvent(new FakeEvent('change'));
 assert.deepEqual(selected,[candidate.videoId]);
 findAll(view,n=>n.tagName==='BUTTON' && n.textContent==='Use this match')[0].click();
 assert.equal(confirms,1);
});
test('metadata failure preserves editable in-app search and announces issue', () => {
 installFakeDom(); let searches=0;
 const view=renderSpotifyMatch(state({stage:'editable',track:{...track,title:null,artists:[]},candidates:[],error:'Metadata unavailable'}),actions({search:()=>searches++}));
 assert.ok(view.textContent.includes('Metadata unavailable'));
 assert.ok(findAll(view,n=>n.tagName==='INPUT' && n.getAttribute('aria-label')==='Search YouTube').length);
 findAll(view,n=>n.tagName==='BUTTON' && n.textContent==='Find on YouTube')[0].click(); assert.equal(searches,1);
});
