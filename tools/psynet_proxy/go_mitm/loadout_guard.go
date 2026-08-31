package main

import (
	"bytes"
	"strings"
)

var loadoutSensitiveServices = []string{
	"products/getloadoutproducts",
	"products/matchcomplete",
	"products/matchcompletefte",
	"products/playerhasloadouttemplate",
	"products/getcontainerdroptable",
	"products/getdestructionproductvalues",
	"products/productupgradelevel",
	"products/schematicstradein",
	"products/unlockcontainer",
	"products/tradein",
	"products/crossentitlement",
	"genericstorage/getplayergenericstorage",
	"genericstorage/setplayergenericstorage",
	"rocketpass/getplayerprestigerewards",
	"rocketpass/getrewardcontent",
	"microtransaction/claimentitlements",
	"microtransaction/getcatalog",
}

func isLoadoutSensitiveService(svc string) bool {
	svc = strings.ToLower(strings.TrimSpace(svc))
	if svc == "" {
		return false
	}
	for _, needle := range loadoutSensitiveServices {
		if strings.Contains(svc, needle) {
			return true
		}
	}
	return false
}

func isLoadoutSensitiveFrame(headers, body []byte) bool {
	if isLoadoutSensitiveService(wsHeaderValue(headers, "PsyService")) {
		return true
	}
	return bodyLooksLoadoutSensitive(body)
}

func bodyLooksLoadoutSensitive(body []byte) bool {
	if len(body) == 0 {
		return false
	}

	for _, cat := range []string{
		"ProfileLoadoutSave_TA",
		"ProductsSave_TA",
		"ExhibitionMatchSettingsSave_TA",
		"PrivateMatchSettingsSave_TA",
	} {
		if bytes.Contains(body, []byte(cat)) {
			return true
		}
	}

	if bytes.Contains(body, []byte("\"bChecksumMatch\"")) && bytes.Contains(body, []byte("\"Category\"")) {
		return true
	}

	if looksLikeInstanceIDLoadout(body) {
		return true
	}
	return false
}

func looksLikeInstanceIDLoadout(body []byte) bool {
	for _, prefix := range [][]byte{
		[]byte(`"Loadout":["`),
		[]byte(`"Loadout": ["`),
	} {
		if bytes.Contains(body, prefix) {
			return true
		}
	}
	return false
}
