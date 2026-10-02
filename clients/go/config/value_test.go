package config

import (
	"encoding/json"
	"testing"
)

func TestRuntimeValuesMarshalWithWireType(t *testing.T) {
	value := Object(map[string]Value{
		"name":    String("sensor"),
		"enabled": Bool(true),
		"levels":  NumericList([]json.Number{"1", "2.5"}),
	})
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	var decoded Value
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded.Type != TypeObject {
		t.Fatalf("type = %q, want object", decoded.Type)
	}
}

func TestIntegerAndNestedTypesDecodeWithoutFloatConversion(t *testing.T) {
	var value Value
	if err := json.Unmarshal([]byte(`{"type":"integer","value":50051}`), &value); err != nil {
		t.Fatal(err)
	}
	if err := value.Validate(); err != nil {
		t.Fatal(err)
	}
	if got := value.Value.(int64); got != 50051 {
		t.Fatalf("integer = %d, want 50051", got)
	}

	var object Value
	if err := json.Unmarshal([]byte(`{"type":"object","value":{"enabled":{"type":"boolean","value":true}}}`), &object); err != nil {
		t.Fatal(err)
	}
	fields := object.Value.(map[string]Value)
	if fields["enabled"].Value != true {
		t.Fatal("nested boolean did not retain its runtime value")
	}
}

func TestRejectsInvalidRuntimeTypes(t *testing.T) {
	if err := (Value{Type: TypeInteger, Value: "not a number"}).Validate(); err == nil {
		t.Fatal("invalid integer was accepted")
	}
	if err := (Value{Type: ValueType("binary"), Value: []byte{1}}).Validate(); err == nil {
		t.Fatal("unknown runtime type was accepted")
	}
}
