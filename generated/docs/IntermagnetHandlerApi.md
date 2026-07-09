# \IntermagnetHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_intermagnet_handler**](IntermagnetHandlerApi.md#create_intermagnet_handler) | **POST** /api/v1/intermagnets | 
[**delete_intermagnet_handler**](IntermagnetHandlerApi.md#delete_intermagnet_handler) | **DELETE** /api/v1/intermagnets/{id} | 
[**get_intermagnet_handler**](IntermagnetHandlerApi.md#get_intermagnet_handler) | **GET** /api/v1/intermagnets/{id} | 
[**list_intermagnet_handler**](IntermagnetHandlerApi.md#list_intermagnet_handler) | **GET** /api/v1/intermagnets | 
[**update_intermagnet_handler**](IntermagnetHandlerApi.md#update_intermagnet_handler) | **PATCH** /api/v1/intermagnets/{id} | 



## create_intermagnet_handler

> models::IntermagnetExtendedModel create_intermagnet_handler(create_intermagnet_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_intermagnet_schema** | [**CreateIntermagnetSchema**](CreateIntermagnetSchema.md) | Create Intermagnet | [required] |

### Return type

[**models::IntermagnetExtendedModel**](IntermagnetExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_intermagnet_handler

> delete_intermagnet_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Id of intermagnet for deletion | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_intermagnet_handler

> models::IntermagnetExtendedModel get_intermagnet_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet id for fetching intermagnet | [required] |

### Return type

[**models::IntermagnetExtendedModel**](IntermagnetExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_intermagnet_handler

> models::PagedResponseIntermagnetExtendedModel list_intermagnet_handler(page, limit, order_by, desc, catalog_id, location_id, publication_state, cadence, orientation)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**catalog_id** | Option<**uuid::Uuid**> | intermagnet catalog id |  |
**location_id** | Option<**uuid::Uuid**> | intermagnet location id |  |
**publication_state** | Option<[**IntermagnetPublicationState**](.md)> | publication state |  |
**cadence** | Option<[**IntermagnetCadence**](.md)> | cadence |  |
**orientation** | Option<[**IntermagnetOrientation**](.md)> | orientation |  |

### Return type

[**models::PagedResponseIntermagnetExtendedModel**](PagedResponse_IntermagnetExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## update_intermagnet_handler

> models::IntermagnetExtendedModel update_intermagnet_handler(id, update_intermagnet_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet id for patching intermagnet | [required] |
**update_intermagnet_schema** | [**UpdateIntermagnetSchema**](UpdateIntermagnetSchema.md) | Update Intermagnet | [required] |

### Return type

[**models::IntermagnetExtendedModel**](IntermagnetExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

