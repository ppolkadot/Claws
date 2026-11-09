import assert from 'node:assert/strict';
import { test } from 'node:test';
import { renderSpotifyMatch, mountSpotifyMatch } from '../../.test-build/views/spotify-match.js';
import { installFakeDom, findAll, FakeEvent, keydown } from './fake-dom.mjs';
const track = {url:'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT',id:'4cOdK2wGLETKBW3PvgPWqT',title:'<script>song</script>',artists:['Artist'],duration:200,artwork:null};
const candidate = {videoId:'dQw4w9WgXcQ',url:'https://www.youtube.com/watch?v=dQw4w9WgXcQ',title:'Song',uploader:'Artist',duration:201,thumbnail:null,score:0.9,warnings:[]};
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
const input = (view) => findAll(view.el ?? view, n => n.tagName === 'INPUT' && n.getAttribute('aria-label') === 'Search YouTube')[0];
const buttons = (view, label) => findAll(view.el ?? view, n => n.tagName === 'BUTTON' && n.textContent === label);
const statusText = (view) => findAll(view.el, n => n.getAttribute('aria-live') === 'polite')[0].textContent;

test('the query field is one node for the whole session, through composition and results', () => {
 installFakeDom(); const edits=[]; let searches=0;
 const view=mountSpotifyMatch(actions({setQuery:q=>edits.push(q),search:()=>searches++}));
 view.update(state({stage:'editable',candidates:[],query:''}));
 const field=input(view); field.focus();
 field.dispatchEvent(new FakeEvent('compositionstart'));
 field.value='夜に'; field.dispatchEvent(new FakeEvent('input'));
 // A state update during composition (e.g. the invalidation echo) must not rewrite the field.
 view.update(state({stage:'editable',candidates:[],query:'夜'}));
 assert.equal(field.value,'夜に');
 assert.equal(input(view),field);
 field.dispatchEvent(keydown('Enter',{isComposing:true}));
 field.dispatchEvent(keydown('Enter'));
 assert.equal(searches,0, 'Enter during composition only commits the IME text');
 field.dispatchEvent(new FakeEvent('compositionend'));
 field.value='夜に駆ける'; field.dispatchEvent(new FakeEvent('input'));
 view.update(state({stage:'editable',candidates:[],query:'夜に駆ける'}));
 field.dispatchEvent(keydown('Enter'));
 assert.equal(searches,1);
 view.update(state()); view.update(state({selectedId:candidate.videoId}));
 view.update(state({stage:'probing',selectedId:candidate.videoId}));
 view.update(state({stage:'confirmed',selectedId:candidate.videoId,confirmed:{}}));
 assert.equal(input(view),field); assert.equal(document.activeElement,field);
 assert.deepEqual(edits,['夜に','夜に駆ける']);
});

test('results and empty outcomes are announced in one persistent polite region', () => {
 installFakeDom(); const view=mountSpotifyMatch(actions());
 view.update(state({stage:'searching',candidates:[]}));
 const region=findAll(view.el,n=>n.getAttribute('aria-live')==='polite')[0];
 assert.match(region.textContent,/Finding/);
 view.update(state());
 assert.equal(statusText(view),'1 recording found. Choose a recording.');
 assert.equal(findAll(view.el,n=>n.tagName==='INPUT'&&n.getAttribute('type')==='radio').some(n=>n.checked),false);
 view.update(state({candidates:[]}));
 assert.match(statusText(view),/No matching recordings/);
 assert.equal(findAll(view.el,n=>n.getAttribute('aria-live')==='polite')[0],region);
});

test('identical warning and error text is shown once; distinct warnings stay in order', () => {
 installFakeDom(); const same="Couldn't read this Spotify track. Enter the artist and song title to search YouTube.";
 const view=renderSpotifyMatch(state({stage:'editable',candidates:[],error:same,warnings:[same,' '+same,'First','Second','First']}),actions());
 assert.equal(view.textContent.split(same).length-1,1);
 const text=view.textContent; assert.ok(text.indexOf('First')<text.indexOf('Second'));
 assert.equal(text.split('First').length-1,1);
});

test('Retry follows retryability, not a missing track', () => {
 installFakeDom(); let retries=0;
 const terminal=renderSpotifyMatch(state({stage:'failed',track:null,candidates:[],error:'Albums are not supported.',retryable:false}),actions({retry:()=>retries++}));
 assert.equal(buttons(terminal,'Try Again').length,0);
 const retryable=renderSpotifyMatch(state({stage:'failed',track:null,candidates:[],error:'Could not resolve.',retryable:true}),actions({retry:()=>retries++}));
 buttons(retryable,'Try Again')[0].click(); assert.equal(retries,1);
 const withTrack=renderSpotifyMatch(state({stage:'editable',candidates:[],error:'x',retryable:false}),actions());
 assert.equal(buttons(withTrack,'Try Again').length,0);
});
