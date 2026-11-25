#!/usr/bin/env bash
# sample.sh <pr-number> <duration-s> <interval-s> <out.jsonl>  -- prints a line whenever the observed state changes
n=$1; dur=$2; iv=$3; out=$4
Q='query($n:Int!,$q:String!){ rateLimit{cost}
 repository(owner:"yachiko",name:"experiment"){ pullRequest(number:$n){ state isDraft updatedAt headRefOid mergeable mergeStateStatus reviewDecision mergedAt comments{totalCount}
   commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:10){nodes{__typename ... on CheckRun{name status conclusion} ... on StatusContext{context state}}}}}}} } }
 viewer{ pullRequests(first:20,states:OPEN,orderBy:{field:UPDATED_AT,direction:DESC}){nodes{number repository{nameWithOwner}}} }
 search(query:$q,type:ISSUE,first:20){ nodes{... on PullRequest{number headRefOid state}} } }'
t0=$(date +%s%N); last=""
while :; do
  ms=$(( ($(date +%s%N) - t0) / 1000000 )); (( ms > dur * 1000 )) && break; el="$((ms/1000)).$(( (ms%1000)/100 ))"
  r=$(gh api graphql -f query="$Q" -F n=$n -f q="is:pr is:open author:@me repo:yachiko/experiment" 2>/dev/null)
  line=$(echo "$r" | jq -c --argjson n $n '.data as $d | $d.repository.pullRequest as $p | {state:$p.state, draft:$p.isDraft, upd:$p.updatedAt, head:($p.headRefOid[0:7]), mergeable:$p.mergeable, mss:$p.mergeStateStatus, comments:$p.comments.totalCount, rollup:($p.commits.nodes[0].commit.statusCheckRollup.state), checks:[$p.commits.nodes[0].commit.statusCheckRollup.contexts.nodes[]? | "\(.name // .context):\(.status // .state):\(.conclusion)"], inViewerOpen:([$d.viewer.pullRequests.nodes[] | select(.number==$n and .repository.nameWithOwner=="yachiko/experiment")]|length>0), inSearchOpen:([$d.search.nodes[] | select(.number==$n)]|length>0), searchHead:([$d.search.nodes[] | select(.number==$n) | .headRefOid[0:7]][0]), cost:$d.rateLimit.cost}')
  if [ "$line" != "$last" ]; then printf '%7ss %s %s\n' "$el" "$(date -u +%H:%M:%S)" "$line" | tee -a "$out"; last="$line"; fi
  sleep "$iv"
done
echo "end $(date -u +%H:%M:%S)" | tee -a "$out"
