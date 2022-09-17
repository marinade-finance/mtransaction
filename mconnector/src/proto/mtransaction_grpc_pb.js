// GENERATED CODE -- DO NOT EDIT!

'use strict';
var grpc = require('@grpc/grpc-js');
var mtransaction_pb = require('./mtransaction_pb.js');

function serialize_validator_EchoRequest(arg) {
  if (!(arg instanceof mtransaction_pb.EchoRequest)) {
    throw new Error('Expected argument of type validator.EchoRequest');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_validator_EchoRequest(buffer_arg) {
  return mtransaction_pb.EchoRequest.deserializeBinary(new Uint8Array(buffer_arg));
}

function serialize_validator_EchoResponse(arg) {
  if (!(arg instanceof mtransaction_pb.EchoResponse)) {
    throw new Error('Expected argument of type validator.EchoResponse');
  }
  return Buffer.from(arg.serializeBinary());
}

function deserialize_validator_EchoResponse(buffer_arg) {
  return mtransaction_pb.EchoResponse.deserializeBinary(new Uint8Array(buffer_arg));
}


// The MTransaction service definition.
var MTransactionService = exports.MTransactionService = {
  // Sends message to be echoed back
echo: {
    path: '/validator.MTransaction/Echo',
    requestStream: false,
    responseStream: false,
    requestType: mtransaction_pb.EchoRequest,
    responseType: mtransaction_pb.EchoResponse,
    requestSerialize: serialize_validator_EchoRequest,
    requestDeserialize: deserialize_validator_EchoRequest,
    responseSerialize: serialize_validator_EchoResponse,
    responseDeserialize: deserialize_validator_EchoResponse,
  },
  // Sends message to be streamed back
echoStream: {
    path: '/validator.MTransaction/EchoStream',
    requestStream: false,
    responseStream: true,
    requestType: mtransaction_pb.EchoRequest,
    responseType: mtransaction_pb.EchoResponse,
    requestSerialize: serialize_validator_EchoRequest,
    requestDeserialize: deserialize_validator_EchoRequest,
    responseSerialize: serialize_validator_EchoResponse,
    responseDeserialize: deserialize_validator_EchoResponse,
  },
};

exports.MTransactionClient = grpc.makeGenericClientConstructor(MTransactionService);
