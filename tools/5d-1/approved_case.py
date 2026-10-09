"""Pin the existing E2E full: case-insensitivity exception; no broad waiver."""
import argparse
import hashlib
import json
import pathlib

from load import semantic


def build(corpus,evidence,nov6):
    if evidence['contract']!='docs/rust/contracts/native-rule-providers.md:57' or evidence['approved_date']!='2026-10-07' or evidence['prior_index']!=3787:
        raise ValueError('not the already approved E2E exception')
    source=evidence['input']; dns=evidence['dns']
    if source['type']!=28 or dns['rcode']!=0 or any(dns[s] for s in ('answers','authority','additional')):
        raise ValueError('expected exact approved empty AAAA answer')
    qname=source['name'].rstrip('.').lower()
    case_rules={line.strip()[5:].rstrip('.').lower() for line in nov6.splitlines()
                if line.strip().startswith('full:') and line.strip()[5:]!=line.strip()[5:].lower()}
    if qname not in case_rules: raise ValueError('snapshot does not contain the approved uppercase full: no-v6 rule')
    matches=[i for i,row in enumerate(corpus) if row['name']==qname and row['type']==28]
    if len(matches)!=1: raise ValueError('approved case must match exactly one corpus cell')
    if dns['questions']!=[[qname+'.',28,1]]: raise ValueError('approved question mismatch')
    return dict(contract=evidence['contract'],approved_date=evidence['approved_date'],prior_index=3787,
                evidence_sha256=hashlib.sha256(json.dumps(evidence,sort_keys=True).encode()).hexdigest(),
                rows=[dict(index=matches[0],semantic=semantic(dns))])


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('corpus','evidence','nov6','output','oracle','native-oracle'): parser.add_argument('--'+key,type=pathlib.Path,required=True)
    args=parser.parse_args()
    evidence=json.loads(args.evidence.read_text())
    provenance=build(json.loads(args.corpus.read_text()),evidence,args.nov6.read_text())
    values=json.loads(args.oracle.read_text())
    index=provenance['rows'][0]['index']
    if values[index]['index']!=index: raise ValueError('oracle index mismatch')
    values[index]['semantic']=provenance['rows'][0]['semantic']
    values[index]['dns']=evidence['dns']
    args.native_oracle.write_text(json.dumps(values))
    provenance['go_oracle_sha256']=hashlib.sha256(args.oracle.read_bytes()).hexdigest()
    provenance['native_oracle_sha256']=hashlib.sha256(args.native_oracle.read_bytes()).hexdigest()
    args.output.write_text(json.dumps(provenance,indent=2))
